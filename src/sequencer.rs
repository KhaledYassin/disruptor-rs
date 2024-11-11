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

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::barrier::ProcessingSequenceBarrier;
use crate::sequence::{AtomicSequence, Sequence};
use crate::traits::Sequencer;
use crate::traits::WaitingStrategy;
use crate::utils::{AvailableSequenceBuffer, Utils};

pub struct SingleProducerSequencer<W: WaitingStrategy> {
    buffer_size: i64,
    cursor: Arc<AtomicSequence>,
    next_value: Cell<Sequence>,
    cached_value: Cell<Sequence>,
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
            next_value: Cell::new(Sequence::from(0)),
            cached_value: Cell::new(Sequence::default()),
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
        let mut min_sequence = self.cached_value.get();
        let next = self.next_value.get();
        let (start, end) = (next, next + (n - 1));

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
        self.next_value.set(end + 1);

        (start, end)
    }

    fn publish(&self, _: Sequence, high: Sequence) {
        self.cursor.set(high);
        self.waiting_strategy.signal_all_when_blocking();
    }

    fn drain(self) {
        let current = self.next_value.get() - 1;
        while Utils::get_minimum_sequence(&self.gating_sequences) < current {
            self.waiting_strategy.signal_all_when_blocking();
        }
        self.is_done.store(true, Ordering::SeqCst);
        self.waiting_strategy.signal_all_when_blocking();
    }
}

impl<W: WaitingStrategy> Drop for SingleProducerSequencer<W> {
    fn drop(&mut self) {
        self.is_done.store(true, Ordering::SeqCst);
        self.waiting_strategy.signal_all_when_blocking();
    }
}

/// A sequencer optimized for multiple producers.
pub struct MultiProducerSequencer<W: WaitingStrategy> {
    buffer_size: i64,
    cursor: Arc<AtomicSequence>,
    high_water_mark: AtomicSequence,
    gating_sequences: Vec<Arc<AtomicSequence>>,
    waiting_strategy: Arc<W>,
    is_done: Arc<AtomicBool>,
    available_buffer: AvailableSequenceBuffer,
}

impl<W: WaitingStrategy> MultiProducerSequencer<W> {
    pub fn new(buffer_size: usize, waiting_strategy: W) -> Self {
        Self {
            buffer_size: buffer_size as i64,
            cursor: Arc::new(AtomicSequence::default()),
            high_water_mark: AtomicSequence::default(),
            gating_sequences: Vec::new(),
            waiting_strategy: Arc::new(waiting_strategy),
            is_done: Arc::new(AtomicBool::new(false)),
            available_buffer: AvailableSequenceBuffer::new(buffer_size as i64),
        }
    }
}

impl<W: WaitingStrategy> Sequencer for MultiProducerSequencer<W> {
    type Barrier = ProcessingSequenceBarrier<W>;

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

    fn next(&self, n: Sequence) -> (Sequence, Sequence) {
        loop {
            let high_water_mark = self.high_water_mark.get();
            if high_water_mark + n - Utils::get_minimum_sequence(&self.gating_sequences)
                < self.buffer_size
            {
                let end = high_water_mark + n;
                if self.high_water_mark.compare_and_set(high_water_mark, end) {
                    return (high_water_mark + 1, end);
                }
            }
        }
    }

    fn publish(&self, low: Sequence, high: Sequence) {
        for sequence in low..=high {
            self.available_buffer.set(sequence);
        }

        let low_water_mark = self.cursor.get() + 1;
        let mut wrapping_point = low_water_mark - 1;

        for sequence in low_water_mark..=self.high_water_mark.get() {
            if self.available_buffer.is_set(sequence) {
                wrapping_point = sequence;
            } else {
                break;
            }
        }

        if wrapping_point > low_water_mark {
            for sequence in low_water_mark..=wrapping_point {
                self.available_buffer.unset(sequence);
            }

            let mut current = low_water_mark;
            while !self.cursor.compare_and_set(current, wrapping_point) {
                current = self.cursor.get();
                if current > wrapping_point {
                    break;
                }
            }
        }

        self.waiting_strategy.signal_all_when_blocking();
    }

    fn drain(self) {
        let current = self.cursor.get();
        while Utils::get_minimum_sequence(&self.gating_sequences) < current {
            self.waiting_strategy.signal_all_when_blocking();
        }
        self.is_done.store(true, Ordering::SeqCst);
        self.waiting_strategy.signal_all_when_blocking();
    }

    fn create_sequence_barrier(&self, gating_sequences: &[Arc<AtomicSequence>]) -> Self::Barrier {
        ProcessingSequenceBarrier::new(
            self.is_done.clone(),
            Vec::from(gating_sequences),
            self.waiting_strategy.clone(),
        )
    }
}

impl<W: WaitingStrategy> Drop for MultiProducerSequencer<W> {
    fn drop(&mut self) {
        self.is_done.store(true, Ordering::SeqCst);
        self.waiting_strategy.signal_all_when_blocking();
    }
}

#[cfg(test)]
mod tests {
    use crate::sequence::AtomicSequence;
    use crate::sequencer::{Sequencer, SingleProducerSequencer};
    use crate::waiting::BusySpinWaitStrategy;
    use std::sync::Arc;
    use std::thread;

    use super::MultiProducerSequencer;

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
    fn test_drain() {
        let sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        sequencer.drain();
    }

    #[test]
    fn test_multi_producer_sequencer_single_consumer() {
        let mut sequencer = MultiProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        let gating_sequence = Arc::new(AtomicSequence::default());
        sequencer.add_gating_sequence(&gating_sequence);

        assert_eq!(sequencer.gating_sequences.len(), 1);
        assert_eq!(sequencer.gating_sequences[0], gating_sequence);

        let (low, high) = sequencer.next(1);
        assert_eq!(low, 0);
        assert_eq!(high, 0);

        sequencer.publish(0, 0);
        gating_sequence.set(0);

        let (low, high) = sequencer.next(BUFFER_SIZE_I64);
        assert_eq!(low, 1);
        assert_eq!(high, BUFFER_SIZE_I64);

        sequencer.publish(1, BUFFER_SIZE_I64);
        gating_sequence.set(BUFFER_SIZE_I64);

        // Test batch publishing with size 8
        let batch_size = 8;
        for i in 0..3 {
            let start = i * batch_size + BUFFER_SIZE_I64 + 1;
            let (low, high) = sequencer.next(batch_size);
            assert_eq!(low, start);
            assert_eq!(high, start + batch_size - 1);

            sequencer.publish(low, high);
            gating_sequence.set(high);
        }

        sequencer.drain();
    }

    #[test]
    fn test_multi_producer_sequencer_multiple_consumers_multi_threaded() {
        let mut sequencer = MultiProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        let num_consumers = 16;
        let num_iterations = 10000;
        let batch_size = 8;

        // Create gating sequences for each consumer
        let gating_sequences: Vec<Arc<AtomicSequence>> = (0..num_consumers)
            .map(|_| Arc::new(AtomicSequence::default()))
            .collect();

        // Add all gating sequences to the sequencer
        for seq in &gating_sequences {
            sequencer.add_gating_sequence(seq);
        }

        // Create consumer threads
        let mut consumer_handles = vec![];
        for gating_sequence in &gating_sequences {
            let gating_sequence = gating_sequence.clone();

            let handle = thread::spawn(move || {
                let mut last_sequence = -1;
                while last_sequence < (num_iterations - 1) {
                    let new_sequence = last_sequence + 1;
                    gating_sequence.set(new_sequence);
                    last_sequence = new_sequence;
                }
            });

            consumer_handles.push(handle);
        }

        // Producer loop
        for i in 0..(num_iterations / batch_size) {
            let (low, high) = sequencer.next(batch_size);
            assert_eq!(low, i * batch_size);
            assert_eq!(high, { i * batch_size + batch_size - 1 });
            sequencer.publish(low, high);
        }

        // Wait for all consumers to complete
        for handle in consumer_handles {
            handle.join().unwrap();
        }

        sequencer.drain();
    }
}
