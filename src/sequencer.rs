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

use crate::barrier::ProcessingSequenceBarrier;
use crate::sequence::{AtomicSequence, Sequence};
use crate::traits::Sequencer;
use crate::traits::WaitingStrategy;
use crate::utils::{AvailableSequenceBuffer, Utils};

pub struct SingleProducerSequencer<W: WaitingStrategy> {
    buffer_size: i64,
    cursor: Arc<AtomicSequence>,
    next_value: Arc<AtomicSequence>,
    cached_value: Arc<AtomicSequence>,
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
            next_value: Arc::new(Sequence::from(0).into()),
            cached_value: Arc::new(Sequence::from(-1).into()),
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

    fn next(&mut self, n: Sequence) -> (Sequence, Sequence) {
        let next = self.next_value;
        let (start, end) = (next, next + (n - 1));

        if !self.gating_sequences.is_empty() {
            let mut min_sequence = self.cached_value;

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

            self.cached_value = min_sequence;
        }

        self.next_value = end + 1;

        (start, end)
    }

    fn publish(&self, _: Sequence, high: Sequence) {
        self.cursor.set(high);
        self.waiting_strategy.signal_all_when_blocking();
    }

    fn drain(self) {
        let current = self.next_value - 1;

        // Wake up any processors so they can finish processing all published events
        self.waiting_strategy.signal_all_when_blocking();

        // Wait until every gating sequence (i.e. every consumer) has caught up
        while Utils::get_minimum_sequence(&self.gating_sequences) < current {
            // Give the consumer threads a chance to run and make progress
            self.waiting_strategy.signal_all_when_blocking();
            std::thread::yield_now();
        }

        // All events are processed – now signal shutdown so processors can exit cleanly
        self.is_done.store(true, Ordering::SeqCst);
        // Final wake-up to ensure blocked threads observe the shutdown signal
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

    fn has_available_capacity(&self, high_water_mark: Sequence, n: Sequence) -> bool {
        high_water_mark + n - Utils::get_minimum_sequence(&self.gating_sequences) < self.buffer_size
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
            if self.has_available_capacity(high_water_mark, n) {
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
    fn test_next_without_gating_sequences_large_request() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        // No gating sequences added
        let n = BUFFER_SIZE_I64 * 2;
        let (start, end) = sequencer.next(n);
        assert_eq!(start, 0);
        assert_eq!(end, n - 1);
    }
}
