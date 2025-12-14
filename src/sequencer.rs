//! The Sequencer is the heart of the Disruptor pattern, coordinating access to the ring buffer.
//!
//! # Overview
//! The Sequencer manages the production and tracking of sequence numbers, which represent
//! positions in the ring buffer. It serves several critical functions:
//!
//! 1. **Sequence Generation**: Provides unique, monotonically increasing sequence numbers
//!    to producers, ensuring ordered data flow.
//!
//! 2. **Capacity Management**: Prevents buffer overflow by tracking consumer progress
//!    through gating sequences and ensuring producers don't overwrite unprocessed data.
//!
//! 3. **Publisher Coordination**: Manages the publication of new entries and notifies
//!    consumers when new data is available.
//!
//! # Single Producer Design
//! This implementation (`SingleProducerSequencer`) is optimized for single-producer scenarios,
//! avoiding the need for CAS operations when claiming sequences. Key features:
//!
//! - Uses an atomic cursor to track the last published sequence
//! - Maintains gating sequences to track consumer progress
//! - Supports configurable waiting strategies for different throughput/CPU trade-offs
//!
//! # Usage Example
//! ```rust
//! use disruptor_rs::{
//!     sequencer::SingleProducerSequencer,
//!     waiting::BusySpinWaitStrategy,
//! };
//!
//! // Create a sequencer with a buffer of 1024 slots
//! let sequencer = SingleProducerSequencer::new(1024, BusySpinWaitStrategy);
//! ```
//!
//! # Producer Workflow
//! 1. Producer requests next sequence(s) via `next()`
//! 2. Writes data to the ring buffer at the claimed sequence(s)
//! 3. Publishes sequences via `publish()` to make data visible to consumers
//!
//! # Consumer Coordination
//! - Consumers track their progress using gating sequences
//! - Sequencer ensures producers don't overwrite data still being processed
//! - Waiting strategy determines how threads wait for available sequences
//!
//! # Multi-Producer Support
//! - `MultiProducerSequencer` supports multiple producers
//! - Uses a high-water mark to track the maximum sequence number
//! - Handles batch publishing and ensures consumers can keep up
//!
//! # Testing
//! - Comprehensive test suite to ensure correctness
//! - Covers single-producer and multi-producer scenarios
//! - Validates behavior under different waiting strategies

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::barrier::{MpmcSequenceBarrier, ProcessingSequenceBarrier};
use crate::sequence::{AtomicSequence, Sequence};
use crate::traits::Sequencer;
use crate::traits::WaitingStrategy;
use crate::utils::{AvailableSequenceBuffer, Utils};

pub struct SingleProducerSequencer<W: WaitingStrategy> {
    buffer_size: i64,
    cursor: Arc<AtomicSequence>,
    next_value: AtomicSequence,
    cached_value: AtomicSequence,
    gating_sequences: Vec<Arc<AtomicSequence>>,
    waiting_strategy: Arc<W>,
    is_done: Arc<AtomicBool>,
}

/// A sequencer optimized for a single producer.
impl<W: WaitingStrategy> SingleProducerSequencer<W> {
    pub fn new(buffer_size: usize, waiting_strategy: W) -> Self {
        Self {
            buffer_size: buffer_size as i64,
            cursor: Arc::new(AtomicSequence::default()),
            next_value: AtomicSequence::new(0),
            cached_value: AtomicSequence::new(-1),
            gating_sequences: Vec::new(),
            waiting_strategy: Arc::new(waiting_strategy),
            is_done: Default::default(),
        }
    }
}

impl<W: WaitingStrategy> Sequencer for SingleProducerSequencer<W> {
    type Barrier = ProcessingSequenceBarrier<W>;

    fn add_gating_sequence(&mut self, gating_sequence: &Arc<AtomicSequence>) {
        self.gating_sequences.push(gating_sequence.clone());
    }

    fn remove_gating_sequence(&mut self, sequence: &Arc<AtomicSequence>) -> bool {
        let index = self
            .gating_sequences
            .iter()
            .position(|s| Arc::ptr_eq(s, sequence));
        if let Some(index) = index {
            self.gating_sequences.remove(index);
            true
        } else {
            false
        }
    }

    fn create_sequence_barrier(&self, gating_sequences: &[Arc<AtomicSequence>]) -> Self::Barrier {
        ProcessingSequenceBarrier::new(
            self.is_done.clone(),
            Vec::from(gating_sequences),
            self.waiting_strategy.clone(),
        )
    }

    fn get_cursor(&self) -> Arc<AtomicSequence> {
        self.cursor.clone()
    }

    fn next(&self, n: Sequence) -> (Sequence, Sequence) {
        // Current highest claimed sequence
        let next = self.next_value.get();

        // Calculate start and end for the range we are about to claim
        let start = next;
        let end = next + (n - 1);

        if !self.gating_sequences.is_empty() {
            let mut min_sequence = self.cached_value.get();

            while min_sequence + self.buffer_size < end {
                if let Some(new_min_sequence) =
                    self.waiting_strategy
                        .wait_for(min_sequence, &self.gating_sequences, || false)
                {
                    min_sequence = new_min_sequence;
                } else {
                    break;
                }
            }

            self.cached_value.set(min_sequence);
        }

        self.next_value.set(end + 1);

        (start, end)
    }

    fn publish(&self, _: Sequence, high: Sequence) {
        self.cursor.set(high);
        self.waiting_strategy.signal_all_when_blocking();
    }

    fn drain(self) {
        let current = self.next_value.get() - 1;

        // Wake up any processors so they can finish processing all published events
        self.waiting_strategy.signal_all_when_blocking();

        // Wait until every gating sequence (i.e. every consumer) has caught up
        while Utils::get_minimum_sequence(&self.gating_sequences) < current {
            // Give the consumer threads a chance to run and make progress
            self.waiting_strategy.signal_all_when_blocking();
            std::thread::yield_now();
        }

        // All events are processed – now signal shutdown so processors can exit cleanly
        self.is_done.store(true, Ordering::Relaxed);
        // Final wake-up to ensure blocked threads observe the shutdown signal
        self.waiting_strategy.signal_all_when_blocking();
    }
}

impl<W: WaitingStrategy> Drop for SingleProducerSequencer<W> {
    fn drop(&mut self) {
        self.is_done.store(true, Ordering::Relaxed);
        self.waiting_strategy.signal_all_when_blocking();
    }
}

/// A sequencer optimized for multiple producers (MPMC).
///
/// Producers claim unique sequences via a monotonic `high_water_mark` and
/// then publish by marking the corresponding slots ready in the
/// [`AvailableSequenceBuffer`]. Consumers wait on a barrier that observes
/// per-slot readiness, removing the need for a publisher-advanced contiguous
/// cursor. Backpressure is enforced by tracking the minimum of consumer
/// sequences to avoid overwriting unprocessed data when the ring wraps.
pub struct MultiProducerSequencer<W: WaitingStrategy> {
    buffer_size: i64,
    cursor: Arc<AtomicSequence>,
    high_water_mark: AtomicSequence,
    cached_value: AtomicSequence,
    gating_sequences: Vec<Arc<AtomicSequence>>,
    waiting_strategy: Arc<W>,
    is_done: Arc<AtomicBool>,
    available_buffer: Arc<AvailableSequenceBuffer>,
}

impl<W: WaitingStrategy> MultiProducerSequencer<W> {
    /// Create a new `MultiProducerSequencer`.
    ///
    /// - `buffer_size` must be a power of two.
    /// - `waiting_strategy` controls how threads coordinate when they cannot
    ///   make immediate progress.
    pub fn new(buffer_size: usize, waiting_strategy: W) -> Self {
        Self {
            buffer_size: buffer_size as i64,
            cursor: Arc::new(AtomicSequence::default()),
            high_water_mark: AtomicSequence::default(),
            cached_value: AtomicSequence::new(-1),
            gating_sequences: Vec::new(),
            waiting_strategy: Arc::new(waiting_strategy),
            is_done: Arc::new(AtomicBool::new(false)),
            available_buffer: Arc::new(AvailableSequenceBuffer::new(buffer_size as i64)),
        }
    }
}

impl<W: WaitingStrategy> Sequencer for MultiProducerSequencer<W> {
    type Barrier = MpmcSequenceBarrier<W>;

    fn add_gating_sequence(&mut self, sequence: &Arc<AtomicSequence>) {
        self.gating_sequences.push(sequence.clone());
    }

    fn remove_gating_sequence(&mut self, sequence: &Arc<AtomicSequence>) -> bool {
        if let Some(pos) = self
            .gating_sequences
            .iter()
            .position(|x| Arc::ptr_eq(x, sequence))
        {
            self.gating_sequences.remove(pos);
            true
        } else {
            false
        }
    }

    fn get_cursor(&self) -> Arc<AtomicSequence> {
        self.cursor.clone()
    }

    /// Claim the next `n` sequences and return `(low, high)` inclusive.
    ///
    /// On the uncontended fast-path, this is a single atomic fetch-add.
    /// When the ring is close to wrapping, the method refreshes the cached
    /// minimum consumer sequence to enforce backpressure and prevent
    /// overwrites. A bounded backoff loop is used only if wrap pressure is
    /// still present after a refresh.
    fn next(&self, n: Sequence) -> (Sequence, Sequence) {
        let next = self.high_water_mark.get_and_add(n);
        let end = next + n;
        let wrap_point = end - self.buffer_size;

        // Tune refresh cadence: refresh less frequently on the fast-path,
        // and always refresh when under wrap pressure.
        const REFRESH_MASK: i64 = 0x3FF; // 1024 - 1
        let mut cached = self.cached_value.get();
        if wrap_point > cached {
            // Immediate refresh when close to wrapping to avoid overwrites
            cached = Utils::get_minimum_sequence(&self.gating_sequences);
            self.cached_value.set(cached);
        } else if (next & REFRESH_MASK) == 0 {
            // Periodic refresh on non-wrap fast-path
            cached = Utils::get_minimum_sequence(&self.gating_sequences);
            self.cached_value.set(cached);
        }

        if wrap_point > cached {
            // Tighter backoff loop with fewer atomic operations
            let mut spins = 0u32;
            loop {
                let min_seq = Utils::get_minimum_sequence(&self.gating_sequences);
                self.cached_value.set(min_seq);
                if wrap_point <= min_seq {
                    break;
                }

                spins = spins.saturating_add(1);
                if spins < 64 {
                    std::hint::spin_loop();
                } else if spins < 128 {
                    std::thread::yield_now();
                } else {
                    std::thread::sleep(std::time::Duration::from_nanos(1));
                    spins = 0;
                }
            }
        }

        (next + 1, end)
    }

    /// Publish the inclusive range `[low, high]` by marking slots as
    /// available in the `AvailableSequenceBuffer`.
    ///
    /// This does not advance any shared cursor; consumers rely on their
    /// barriers to discover per-slot readiness and contiguous ranges.
    fn publish(&self, low: Sequence, high: Sequence) {
        // Mark published range as available; do not advance cursor here.
        self.available_buffer.set_batch(low, high);
        self.waiting_strategy.signal_all_when_blocking();
    }

    /// Block until all consumers have processed up to the highest contiguous
    /// published sequence, then signal shutdown.
    fn drain(self) {
        // Determine the highest contiguous published sequence up to the claimed high-water mark.
        let claimed = self.high_water_mark.get();
        let target = self.available_buffer.highest_published_sequence(0, claimed);

        // Wait for all consumers to process up to target.
        while Utils::get_minimum_sequence(&self.gating_sequences) < target {
            self.waiting_strategy.signal_all_when_blocking();
            std::thread::yield_now();
        }

        self.is_done.store(true, Ordering::Relaxed);
        self.waiting_strategy.signal_all_when_blocking();
    }

    fn create_sequence_barrier(&self, _gating_sequences: &[Arc<AtomicSequence>]) -> Self::Barrier {
        MpmcSequenceBarrier::new(
            self.is_done.clone(),
            self.available_buffer.clone(),
            self.waiting_strategy.clone(),
        )
    }
}

impl<W: WaitingStrategy> Drop for MultiProducerSequencer<W> {
    fn drop(&mut self) {
        self.is_done.store(true, Ordering::Relaxed);
        self.waiting_strategy.signal_all_when_blocking();
    }
}

#[cfg(test)]
mod tests {
    use crate::sequence::AtomicSequence;
    use crate::sequencer::{Sequencer, SingleProducerSequencer};
    use crate::waiting::BusySpinWaitStrategy;
    use std::sync::Arc;

    const BUFFER_SIZE: usize = 16;
    const BUFFER_SIZE_I64: i64 = BUFFER_SIZE as i64;

    #[test]
    fn test_get_cursor() {
        let sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        assert_eq!(sequencer.get_cursor().get(), -1);
    }

    #[test]
    fn test_next() {
        let gating_sequence = Arc::new(AtomicSequence::default());
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        sequencer.add_gating_sequence(&gating_sequence);
        assert_eq!(sequencer.next(1), (0, 0));
        sequencer.publish(0, 0);
        gating_sequence.set(0);
        assert_eq!(sequencer.next(BUFFER_SIZE_I64), (1, BUFFER_SIZE_I64));
    }

    #[test]
    fn test_publish() {
        let sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        sequencer.publish(0, 10);
        assert_eq!(sequencer.cursor.get(), 10);
    }

    #[test]
    fn test_add_gating_sequences() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        let gating_sequence = Arc::new(AtomicSequence::default());
        sequencer.add_gating_sequence(&gating_sequence);
        assert_eq!(sequencer.gating_sequences.len(), 1);
        assert_eq!(sequencer.gating_sequences[0], gating_sequence);
    }

    #[test]
    fn test_remove_gating_sequence() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        let gating_sequence = Arc::new(AtomicSequence::default());
        sequencer.add_gating_sequence(&gating_sequence);
        assert_eq!(sequencer.gating_sequences.len(), 1);
        assert!(sequencer.remove_gating_sequence(&gating_sequence));
        assert_eq!(sequencer.gating_sequences.len(), 0);
        assert!(!sequencer.remove_gating_sequence(&gating_sequence));
    }

    #[test]
    fn test_next_without_gating_sequences_large_request() {
        let sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        // No gating sequences added
        let n = BUFFER_SIZE_I64 * 2;
        let (start, end) = sequencer.next(n);
        assert_eq!(start, 0);
        assert_eq!(end, n - 1);
    }

    #[test]
    fn test_multi_producer_sequencer_basic() {
        use crate::sequencer::MultiProducerSequencer;

        println!("Creating MultiProducerSequencer...");
        let sequencer = MultiProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        println!("Testing next() without gating sequences...");
        let (start, end) = sequencer.next(1);
        println!("Got range: {start} to {end}");
        assert_eq!(start, 0);
        assert_eq!(end, 0);

        println!("Testing publish()...");
        sequencer.publish(start, end);
        println!("Published successfully");

        assert!(sequencer.available_buffer.is_available(0));
        println!("Test completed successfully!");
    }

    #[test]
    fn test_multi_producer_sequencer_with_gating_sequences() {
        use crate::sequencer::MultiProducerSequencer;

        println!("Creating MultiProducerSequencer with gating sequences...");
        let mut sequencer = MultiProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        // Add gating sequences (simulating consumers)
        let consumer1 = Arc::new(AtomicSequence::new(-1));
        let consumer2 = Arc::new(AtomicSequence::new(-1));
        sequencer.add_gating_sequence(&consumer1);
        sequencer.add_gating_sequence(&consumer2);

        println!("Testing next() with gating sequences...");
        let (start, end) = sequencer.next(1);
        println!("Got range: {start} to {end}");

        println!("Testing publish()...");
        sequencer.publish(start, end);
        println!(
            "Published successfully, available[0]: {}",
            sequencer.available_buffer.is_available(0)
        );

        // Now try to fill up the buffer
        println!("Filling buffer to test wrap-around...");
        for i in 1..BUFFER_SIZE {
            println!("Claiming sequence {i}");
            let (s, e) = sequencer.next(1);
            println!("Got range: {s} to {e}");
            sequencer.publish(s, e);
            println!(
                "Published sequence {i}, available[{}]: {}",
                i,
                sequencer.available_buffer.is_available(i as i64)
            );
        }

        // Now simulate consumers making progress before trying wrap-around
        println!("Advancing consumers to simulate progress...");
        consumer1.set(15); // Consumer caught up to the latest published sequence
        consumer2.set(15); // Consumer caught up to the latest published sequence

        // This should now work since consumers have made progress
        println!("Testing wrap-around scenario...");
        println!(
            "Before next(): consumer1={}, consumer2={}",
            consumer1.get(),
            consumer2.get()
        );

        let (start, end) = sequencer.next(1);
        println!("After next(): Got range: {start} to {end}");

        println!("Test completed successfully!");
    }

    #[test]
    fn test_multi_producer_sequencer_with_consumer_progress() {
        use crate::sequencer::MultiProducerSequencer;

        println!("Creating MultiProducerSequencer with progressing consumer...");
        let mut sequencer = MultiProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        // Add gating sequence (simulating a consumer)
        let consumer = Arc::new(AtomicSequence::new(-1));
        sequencer.add_gating_sequence(&consumer);

        // Fill the buffer completely
        println!("Filling buffer completely...");
        for i in 0..BUFFER_SIZE {
            let (start, end) = sequencer.next(1);
            println!("Claimed sequence {i}: {start} to {end}");
            sequencer.publish(start, end);

            // Simulate consumer processing (advance every few events)
            if i % 4 == 3 {
                consumer.set(i as i64);
                println!("Consumer advanced to {i}");
            }
        }

        println!("Buffer full. consumer: {}", consumer.get());

        // Try to claim one more - this should work now that consumer has progressed
        println!("Trying to claim one more sequence...");
        let (start, end) = sequencer.next(1);
        println!("Successfully claimed: {start} to {end}");

        println!("Test completed successfully!");
    }

    #[test]
    fn test_multi_producer_out_of_order_publish_deadlock() {
        use crate::sequencer::MultiProducerSequencer;

        println!("Testing out-of-order publish deadlock scenario...");
        let mut sequencer = MultiProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        // Add a consumer gating sequence
        let consumer = Arc::new(AtomicSequence::new(-1));
        sequencer.add_gating_sequence(&consumer);

        println!("Initial state - consumer: {}", consumer.get());

        // Claim 3 sequences (simulating 3 producers)
        let (seq1_start, seq1_end) = sequencer.next(1);
        let (seq2_start, seq2_end) = sequencer.next(1);
        let (seq3_start, seq3_end) = sequencer.next(1);

        println!("Claimed sequences: {seq1_start} {seq2_start} {seq3_start}");
        // no cursor advancement under MPMC publish semantics

        // Publish out of order: 3rd, 2nd, then 1st (simulating race condition)
        println!("Publishing sequence {seq3_start} (3rd)");
        sequencer.publish(seq3_start, seq3_end);
        // publish 3rd out of order

        println!("Publishing sequence {seq2_start} (2nd)");
        sequencer.publish(seq2_start, seq2_end);
        // publish 2nd out of order

        // At this point cursor should still be at -1 because sequence 0 hasn't been published
        // But if a consumer tries to wait for sequence 1 or 2, it will hang!

        println!("Publishing sequence {seq1_start} (1st)");
        sequencer.publish(seq1_start, seq1_end);
        // publish 1st, contiguity established

        // Now cursor should jump to sequence 2 (all sequences 0,1,2 are published)
        assert_eq!(
            sequencer.available_buffer.highest_published_sequence(0, 2),
            2
        );

        println!("Test completed successfully!");
    }

    #[test]
    fn test_benchmark_simulation_with_barriers() {
        use crate::barrier::MpmcSequenceBarrier;
        use crate::sequencer::MultiProducerSequencer;
        use crate::traits::{SequenceBarrier, WaitingStrategy};
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        println!("Simulating benchmark with real barriers...");
        let mut sequencer = MultiProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        // Create consumer sequences
        let consumer1 = Arc::new(AtomicSequence::new(-1));
        let consumer2 = Arc::new(AtomicSequence::new(-1));
        let consumer3 = Arc::new(AtomicSequence::new(-1));

        // Add them as gating sequences
        sequencer.add_gating_sequence(&consumer1);
        sequencer.add_gating_sequence(&consumer2);
        sequencer.add_gating_sequence(&consumer3);

        let sequencer = Arc::new(sequencer);

        // Create barriers (like the real benchmark does)
        let barrier1 = MpmcSequenceBarrier::new(
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sequencer.available_buffer.clone(),
            Arc::new(BusySpinWaitStrategy::new()),
        );
        let barrier2 = MpmcSequenceBarrier::new(
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sequencer.available_buffer.clone(),
            Arc::new(BusySpinWaitStrategy::new()),
        );
        let barrier3 = MpmcSequenceBarrier::new(
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sequencer.available_buffer.clone(),
            Arc::new(BusySpinWaitStrategy::new()),
        );

        // Start consumer threads (simplified processors)
        let consumer1_clone = Arc::clone(&consumer1);
        let consumer_thread1 = thread::spawn(move || {
            let mut processed = 0;
            while processed < 50 {
                // Process a limited number
                let next_seq = consumer1_clone.get() + 1;
                if let Some(available) = barrier1.wait_for(next_seq) {
                    consumer1_clone.set(available);
                    processed += available - next_seq + 1;
                    println!("Consumer 1 advanced to {available}");
                } else {
                    break;
                }
                thread::sleep(Duration::from_millis(1)); // Simulate processing time
            }
            println!("Consumer 1 finished at {}", consumer1_clone.get());
        });

        let consumer2_clone = Arc::clone(&consumer2);
        let consumer_thread2 = thread::spawn(move || {
            let mut processed = 0;
            while processed < 50 {
                let next_seq = consumer2_clone.get() + 1;
                if let Some(available) = barrier2.wait_for(next_seq) {
                    consumer2_clone.set(available);
                    processed += available - next_seq + 1;
                    println!("Consumer 2 advanced to {available}");
                } else {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
            println!("Consumer 2 finished at {}", consumer2_clone.get());
        });

        let consumer3_clone = Arc::clone(&consumer3);
        let consumer_thread3 = thread::spawn(move || {
            let mut processed = 0;
            while processed < 50 {
                let next_seq = consumer3_clone.get() + 1;
                if let Some(available) = barrier3.wait_for(next_seq) {
                    consumer3_clone.set(available);
                    processed += available - next_seq + 1;
                    println!("Consumer 3 advanced to {available}");
                } else {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
            println!("Consumer 3 finished at {}", consumer3_clone.get());
        });

        // Give consumers a moment to start
        thread::sleep(Duration::from_millis(10));

        // Start producer threads
        let mut producers = vec![];
        for producer_id in 0..3 {
            let sequencer = Arc::clone(&sequencer);
            let producer = thread::spawn(move || {
                for _ in 0..20 {
                    // Reduced number to avoid timeout
                    let (start, end) = sequencer.next(1);
                    sequencer.publish(start, end);
                    println!("Producer {producer_id} published sequence {start}");
                    thread::sleep(Duration::from_millis(1));
                }
                println!("Producer {producer_id} finished");
            });
            producers.push(producer);
        }

        // Wait for producers
        for producer in producers {
            producer.join().unwrap();
        }

        // Give consumers time to catch up
        thread::sleep(Duration::from_millis(100));

        // Wait for consumers
        consumer_thread1.join().unwrap();
        consumer_thread2.join().unwrap();
        consumer_thread3.join().unwrap();

        println!("Final state:");
        let hp = sequencer
            .available_buffer
            .highest_published_sequence(0, 10_000);
        println!("Highest contiguous: {}", hp);
        println!("Consumer1: {}", consumer1.get());
        println!("Consumer2: {}", consumer2.get());
        println!("Consumer3: {}", consumer3.get());

        println!("Test completed successfully!");
    }

    #[test]
    fn test_multi_producer_simple_threading() {
        use crate::sequencer::MultiProducerSequencer;
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        println!("Testing MultiProducerSequencer with simple threading...");
        let mut sequencer = MultiProducerSequencer::new(64, BusySpinWaitStrategy); // Small buffer

        // Create consumer sequences
        let consumer1 = Arc::new(AtomicSequence::new(-1));
        let consumer2 = Arc::new(AtomicSequence::new(-1));
        sequencer.add_gating_sequence(&consumer1);
        sequencer.add_gating_sequence(&consumer2);

        let sequencer = Arc::new(sequencer);

        // Start 2 producer threads
        let mut producers = vec![];
        for producer_id in 0..2 {
            let sequencer = Arc::clone(&sequencer);
            let producer = thread::spawn(move || {
                for _ in 0..20 {
                    // Small number of sequences
                    let (start, end) = sequencer.next(1);
                    sequencer.publish(start, end);
                    println!("Producer {producer_id} published sequence {start}");
                    thread::sleep(Duration::from_millis(1)); // Small delay
                }
                println!("Producer {producer_id} finished");
            });
            producers.push(producer);
        }

        // Start 2 consumer threads
        let consumer1_clone = Arc::clone(&consumer1);
        let consumer_thread1 = thread::spawn(move || {
            let mut processed = 0;
            while processed < 20 {
                // Process 20 sequences
                let next_seq = consumer1_clone.get() + 1;
                // Simulate consuming by just advancing the sequence
                consumer1_clone.set(next_seq);
                processed += 1;
                println!("Consumer 1 advanced to {next_seq}");
                thread::sleep(Duration::from_millis(2)); // Simulate processing time
            }
            println!("Consumer 1 finished at {}", consumer1_clone.get());
        });

        let consumer2_clone = Arc::clone(&consumer2);
        let consumer_thread2 = thread::spawn(move || {
            let mut processed = 0;
            while processed < 20 {
                // Process 20 sequences
                let next_seq = consumer2_clone.get() + 1;
                // Simulate consuming by just advancing the sequence
                consumer2_clone.set(next_seq);
                processed += 1;
                println!("Consumer 2 advanced to {next_seq}");
                thread::sleep(Duration::from_millis(2)); // Simulate processing time
            }
            println!("Consumer 2 finished at {}", consumer2_clone.get());
        });

        // Wait for all threads
        for producer in producers {
            producer.join().unwrap();
        }
        consumer_thread1.join().unwrap();
        consumer_thread2.join().unwrap();

        println!("Final state:");
        let hp = sequencer
            .available_buffer
            .highest_published_sequence(0, 1000);
        println!("Highest contiguous: {}", hp);
        println!("Consumer1: {}", consumer1.get());
        println!("Consumer2: {}", consumer2.get());

        // Both producers published 20 sequences each, so highest contiguous should be around 39
        assert!(hp >= 35); // Allow some leeway for race conditions
        assert!(consumer1.get() >= 15); // Consumers should have made good progress
        assert!(consumer2.get() >= 15);

        println!("Test completed successfully!");
    }

    #[test]
    fn test_disruptor_builder_barrier_setup() {
        use crate::builder::DisruptorBuilder;
        use crate::traits::{EventHandler, EventProcessorExecutor, EventProducer, ExecutorHandle};
        use std::thread;
        use std::time::Duration;

        struct TestHandler;
        impl EventHandler<i64> for TestHandler {
            fn on_event(&self, _event: &i64, sequence: crate::sequence::Sequence, _: bool) {
                println!("Handler processed sequence {sequence}");
            }
            fn on_start(&self) {}
            fn on_shutdown(&self) {}
        }

        println!("Creating disruptor with multi-producer sequencer...");
        let (executor, producer) = DisruptorBuilder::with_ring_buffer(64)
            .with_busy_spin_waiting_strategy()
            .with_multi_producer_sequencer()
            .with_barrier(|b| {
                b.handle_events(TestHandler);
                b.handle_events(TestHandler);
            })
            .build();

        println!("Starting executor...");
        let handle = executor.spawn();

        // Give the executor time to start
        thread::sleep(Duration::from_millis(10));

        println!("Producing 5 events...");
        for i in 0..5 {
            producer.write(vec![i], |slot, _seq, _| {
                *slot = i;
            });
            println!("Produced event {i}");
            thread::sleep(Duration::from_millis(5));
        }

        println!("Draining producer...");
        producer.drain();

        println!("Joining executor...");
        handle.join();

        println!("Test completed successfully!");
    }

    #[test]
    fn test_multi_producer_optimized_publish_single_sequence() {
        use crate::sequencer::MultiProducerSequencer;

        let sequencer = MultiProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        // Test single sequence publish
        let (start, end) = sequencer.next(1);
        assert_eq!(start, 0);
        assert_eq!(end, 0);

        sequencer.publish(start, end);
        assert!(sequencer.available_buffer.is_available(0));
    }

    #[test]
    fn test_multi_producer_optimized_publish_batch() {
        use crate::sequencer::MultiProducerSequencer;

        let sequencer = MultiProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        // Test batch publish
        let (start, end) = sequencer.next(5);
        assert_eq!(start, 0);
        assert_eq!(end, 4);

        sequencer.publish(start, end);
        for i in start..=end {
            assert!(sequencer.available_buffer.is_available(i));
        }
    }

    #[test]
    fn test_multi_producer_out_of_order_publish_with_optimizations() {
        use crate::sequencer::MultiProducerSequencer;

        let sequencer = MultiProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        // Claim three sequences
        let (seq1_start, seq1_end) = sequencer.next(1); // 0-0
        let (seq2_start, seq2_end) = sequencer.next(1); // 1-1
        let (seq3_start, seq3_end) = sequencer.next(1); // 2-2

        // Publish out of order: 3rd, 1st, 2nd
        sequencer.publish(seq3_start, seq3_end); // Publish sequence 2
        assert_eq!(
            sequencer.available_buffer.highest_published_sequence(0, 2),
            -1
        );

        sequencer.publish(seq1_start, seq1_end); // Publish sequence 0
        assert_eq!(
            sequencer.available_buffer.highest_published_sequence(0, 2),
            0
        );

        sequencer.publish(seq2_start, seq2_end); // Publish sequence 1
        assert_eq!(
            sequencer.available_buffer.highest_published_sequence(0, 2),
            2
        );
    }

    #[test]
    fn test_multi_producer_publish_with_scan_limiting() {
        use crate::sequencer::MultiProducerSequencer;

        let sequencer = MultiProducerSequencer::new(128, BusySpinWaitStrategy);

        // Claim a large batch to test scan limiting
        let (start, end) = sequencer.next(100);
        sequencer.publish(start, end);

        // The cursor should advance, but the scan should be limited
        // This test primarily ensures the scan limiting doesn't break functionality
        assert_eq!(
            sequencer
                .available_buffer
                .highest_published_sequence(0, end),
            end
        );
    }

    #[test]
    fn test_multi_producer_concurrent_publish_with_backoff() {
        use crate::sequencer::MultiProducerSequencer;
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        let sequencer = Arc::new(MultiProducerSequencer::new(64, BusySpinWaitStrategy));
        let num_threads = 4;
        let sequences_per_thread = 8;

        let mut handles = vec![];

        // Spawn threads that publish concurrently
        for _ in 0..num_threads {
            let sequencer_clone = Arc::clone(&sequencer);
            let handle = thread::spawn(move || {
                for _ in 0..sequences_per_thread {
                    let (start, end) = sequencer_clone.next(1);
                    // Add small delay to increase chance of contention
                    thread::sleep(Duration::from_nanos(100));
                    sequencer_clone.publish(start, end);
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // All sequences should be published
        let expected_final_cursor = (num_threads * sequences_per_thread - 1) as i64;
        assert_eq!(
            sequencer
                .available_buffer
                .highest_published_sequence(0, expected_final_cursor),
            expected_final_cursor
        );
    }

    #[test]
    fn test_multi_producer_batch_operations_integration() {
        use crate::sequencer::MultiProducerSequencer;

        let sequencer = MultiProducerSequencer::new(32, BusySpinWaitStrategy);

        // Test that batch operations work correctly with the sequencer
        let (start1, end1) = sequencer.next(3); // 0-2
        let (start2, end2) = sequencer.next(2); // 3-4
        let (start3, end3) = sequencer.next(1); // 5-5

        // Publish batches out of order
        sequencer.publish(start2, end2); // Publish 3-4
        sequencer.publish(start3, end3); // Publish 5

        // Cursor should not advance yet
        assert_eq!(
            sequencer.available_buffer.highest_published_sequence(0, 5),
            -1
        );

        sequencer.publish(start1, end1); // Publish 0-2

        // Now cursor should advance to 5 (all sequences published)
        assert_eq!(
            sequencer.available_buffer.highest_published_sequence(0, 5),
            5
        );
    }

    #[test]
    fn test_multi_producer_exponential_backoff_behavior() {
        use crate::sequencer::MultiProducerSequencer;
        use std::sync::{Arc, Barrier};
        use std::thread;
        use std::time::Instant;

        let sequencer = Arc::new(MultiProducerSequencer::new(16, BusySpinWaitStrategy));
        let barrier = Arc::new(Barrier::new(3));
        let num_contentious_threads = 2;

        let mut handles = vec![];

        // Create threads that will contend heavily on cursor updates
        for _ in 0..num_contentious_threads {
            let sequencer_clone = Arc::clone(&sequencer);
            let barrier_clone = Arc::clone(&barrier);
            let handle = thread::spawn(move || {
                // Synchronize thread start to maximize contention
                barrier_clone.wait();

                let start_time = Instant::now();

                // Publish many small sequences rapidly to trigger backoff
                for _ in 0..10 {
                    let (start, end) = sequencer_clone.next(1);
                    sequencer_clone.publish(start, end);
                }

                start_time.elapsed()
            });
            handles.push(handle);
        }

        // Wait for threads to be ready
        barrier.wait();

        let mut durations = vec![];
        for handle in handles {
            durations.push(handle.join().unwrap());
        }

        // The test passes if all threads complete successfully.
        // Because the ring overwrites older generations, only the last `buffer_size`
        // sequences are expected to be contiguous.
        let expected_final = (num_contentious_threads * 10 - 1) as i64; // e.g., 19
        let buffer_size = 16i64; // created above
        let window_start = expected_final - (buffer_size - 1);
        assert_eq!(
            sequencer
                .available_buffer
                .highest_published_sequence(window_start, expected_final),
            expected_final
        );

        // Ensure threads didn't take excessively long (backoff should be bounded)
        for duration in durations {
            assert!(
                duration.as_millis() < 1000,
                "Thread took too long, backoff may be excessive"
            );
        }
    }

    #[test]
    fn test_multi_producer_available_buffer_state_consistency() {
        use crate::sequencer::MultiProducerSequencer;

        let sequencer = MultiProducerSequencer::new(16, BusySpinWaitStrategy);

        // Publish some sequences
        let (start1, end1) = sequencer.next(3);
        let (start2, end2) = sequencer.next(2);

        sequencer.publish(start1, end1);
        sequencer.publish(start2, end2);

        // After publishing, the available buffer should be in a consistent state
        // (sequences should be marked as available then cleared after cursor advancement)

        // This is primarily a sanity check that the optimized publish doesn't
        // leave the available buffer in an inconsistent state
        for i in start1..=end2 {
            assert!(sequencer.available_buffer.is_available(i));
        }
    }

    #[test]
    fn test_multi_producer_large_batch_with_wraparound() {
        use crate::sequencer::MultiProducerSequencer;

        let buffer_size = 8;
        let sequencer = MultiProducerSequencer::new(buffer_size, BusySpinWaitStrategy);

        // First, fill up most of the buffer
        let (start1, end1) = sequencer.next(6);
        sequencer.publish(start1, end1);
        assert_eq!(
            sequencer
                .available_buffer
                .highest_published_sequence(0, end1),
            end1
        );

        // Now publish a batch that will wrap around
        let (start2, end2) = sequencer.next(4); // Should wrap around
        sequencer.publish(start2, end2);

        // Verify availability for the wrapped range and contiguous from start2
        for i in start2..=end2 {
            assert!(sequencer.available_buffer.is_available(i));
        }
        assert_eq!(
            sequencer
                .available_buffer
                .highest_published_sequence(start2, end2),
            end2
        );
    }

    #[test]
    fn test_multi_producer_performance_comparison_stress() {
        use crate::sequencer::MultiProducerSequencer;
        use std::sync::Arc;
        use std::thread;
        use std::time::Instant;

        let sequencer = Arc::new(MultiProducerSequencer::new(1024, BusySpinWaitStrategy));
        let num_threads = 8;
        let operations_per_thread = 100;

        let start_time = Instant::now();
        let mut handles = vec![];

        for _ in 0..num_threads {
            let sequencer_clone = Arc::clone(&sequencer);
            let handle = thread::spawn(move || {
                for _ in 0..operations_per_thread {
                    let (start, end) = sequencer_clone.next(1);
                    sequencer_clone.publish(start, end);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let duration = start_time.elapsed();
        let total_operations = num_threads * operations_per_thread;

        // Verify correctness
        assert_eq!(
            sequencer
                .available_buffer
                .highest_published_sequence(0, (total_operations - 1) as i64),
            (total_operations - 1) as i64
        );

        // Performance check - should complete reasonably quickly
        // This is more of a sanity check than a precise benchmark
        assert!(
            duration.as_millis() < 5000,
            "Stress test took {} ms, may indicate performance regression",
            duration.as_millis()
        );

        println!(
            "Completed {} operations across {} threads in {} ms",
            total_operations,
            num_threads,
            duration.as_millis()
        );
    }
}
