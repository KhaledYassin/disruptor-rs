use std::sync::Arc;

use crate::barrier::{ProcessingSequenceBarrier, SequenceBarrier};
use crate::sequence::{AtomicSequence, Sequence};
use crate::utils::Utils;
use crate::waiting::WaitingStrategy; // Import the SequenceBarrier type

pub trait Sequencer {
    type Barrier: SequenceBarrier;
    // Inteferface methods
    fn claim(&mut self, sequence: Sequence);
    fn is_available(&self, sequence: Sequence) -> bool;
    fn add_gating_sequences(&mut self, gating_sequence: Arc<AtomicSequence>);
    fn remove_gating_sequence(&mut self, sequence: Arc<AtomicSequence>) -> bool;
    fn create_sequence_barrier(&self) -> Self::Barrier;

    // Abstract methods
    fn get_cursor(&self) -> Sequence;
    fn get_buffer_size(&self) -> i64;
    fn has_available_capacity(&mut self, required_capacity: Sequence) -> bool;
    fn get_remaining_capacity(&self) -> Sequence;
    fn next_one(&mut self) -> Option<Sequence> {
        self.next(1)
    }
    fn next(&mut self, n: Sequence) -> Option<Sequence>;
    fn publish(&self, low: Sequence, high: Sequence);
}

pub struct SingleProducerSequencer<W: WaitingStrategy> {
    buffer_size: i64,
    mask: i64,
    cursor: Arc<AtomicSequence>,
    next_value: Sequence,
    cached_value: Sequence,
    gating_sequences: Vec<Arc<AtomicSequence>>,
    waiting_strategy: Arc<W>,
}

impl<W: WaitingStrategy> SingleProducerSequencer<W> {
    pub fn new(buffer_size: usize, waiting_strategy: W) -> Self {
        let buffer_size = buffer_size as i64;
        let mask = buffer_size - 1;
        let cursor = Arc::new(AtomicSequence::default());
        let gating_sequences = Vec::new();
        Self {
            buffer_size,
            mask,
            cursor,
            next_value: Sequence::from(-1),
            cached_value: Sequence::from(-1),
            gating_sequences,
            waiting_strategy: Arc::new(waiting_strategy),
        }
    }
}

impl<W: WaitingStrategy> Sequencer for SingleProducerSequencer<W> {
    type Barrier = ProcessingSequenceBarrier<W>;

    fn claim(&mut self, sequence: Sequence) {
        self.next_value = sequence;
    }

    fn is_available(&self, sequence: Sequence) -> bool {
        let current_sequence = self.cursor.get();
        sequence <= current_sequence && sequence > current_sequence - self.buffer_size
    }

    fn add_gating_sequences(&mut self, gating_sequence: Arc<AtomicSequence>) {
        self.gating_sequences.push(gating_sequence);
    }

    fn remove_gating_sequence(&mut self, sequence: Arc<AtomicSequence>) -> bool {
        let index = self
            .gating_sequences
            .iter()
            .position(|s| Arc::ptr_eq(s, &sequence));
        if let Some(index) = index {
            self.gating_sequences.remove(index);
            true
        } else {
            false
        }
    }

    fn create_sequence_barrier(&self) -> Self::Barrier {
        ProcessingSequenceBarrier::new(
            self.cursor.clone(),
            Arc::new(false.into()),
            self.gating_sequences.clone(),
            self.waiting_strategy.clone(),
        )
    }

    fn get_cursor(&self) -> Sequence {
        self.cursor.get()
    }

    fn get_buffer_size(&self) -> i64 {
        self.buffer_size
    }

    fn has_available_capacity(&mut self, required_capacity: Sequence) -> bool {
        let next_value = self.next_value;
        let wrap_point = next_value + required_capacity - self.buffer_size;
        let cached_value = self.cached_value;

        if wrap_point > cached_value || cached_value > next_value {
            let min_sequence = Utils::get_minimum_sequence(&self.gating_sequences, next_value);
            self.cached_value = min_sequence;

            if wrap_point > min_sequence {
                return false;
            }
        }

        true
    }

    fn get_remaining_capacity(&self) -> Sequence {
        let next_value = self.next_value;
        let consumed = Utils::get_minimum_sequence(&self.gating_sequences, next_value);
        let produced = next_value;

        self.buffer_size - (produced - consumed)
    }

    fn next(&mut self, n: Sequence) -> Option<Sequence> {
        let next = self.next_value;
        let next_sequence = next + n;
        let wrap_point = next_sequence.wrapping_sub(self.buffer_size);
        let cached_value = self.cached_value;

        if wrap_point > cached_value || cached_value > next {
            self.cursor.set(next_sequence);

            let mut min_sequence = Utils::get_minimum_sequence(&self.gating_sequences, next);

            if wrap_point > min_sequence {
                if let Some(sequence) =
                    self.waiting_strategy
                        .wait_for(&self.cursor, &self.gating_sequences, || {
                            self.cursor.get() > wrap_point
                        })
                {
                    min_sequence = sequence;
                } else {
                    return None;
                }
            }

            self.cached_value = min_sequence;
        }

        self.next_value = next_sequence;

        Some(next_sequence)
    }

    fn publish(&self, _: Sequence, high: Sequence) {
        self.cursor.set(high);
        self.waiting_strategy.signal_all_when_blocking();
    }

    fn next_one(&mut self) -> Option<Sequence> {
        self.next(1)
    }
}

#[cfg(test)]
mod tests {
    use crate::waiting::BusySpinWaitStrategy;

    use super::*;

    const BUFFER_SIZE: usize = 16;
    const BUFFER_SIZE_I64: i64 = BUFFER_SIZE as i64;

    #[test]
    fn test_claim() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        assert_eq!(sequencer.next_one(), Some(0));
    }

    #[test]
    fn test_next_one() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        sequencer.claim(0);
        assert_eq!(sequencer.next_one(), Some(1));
    }

    #[test]
    fn test_is_available() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        if let Some(next) = sequencer.next(6) {
            for i in 0..6 {
                assert!(!sequencer.is_available(i));
            }

            sequencer.publish(next - (6 - 1), next);

            for i in 0..6 {
                assert!(sequencer.is_available(i));
            }

            assert!(!sequencer.is_available(6));
        } else {
            panic!("Expected a value but got None.");
        }
    }

    #[test]
    fn test_add_gating_sequences() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        let gating_sequence = Arc::new(AtomicSequence::default());
        sequencer.add_gating_sequences(gating_sequence.clone());
        assert_eq!(sequencer.gating_sequences.len(), 1);
        assert_eq!(sequencer.gating_sequences[0], gating_sequence);
    }

    #[test]
    fn test_remove_gating_sequence() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        let gating_sequence = Arc::new(AtomicSequence::default());
        sequencer.add_gating_sequences(gating_sequence.clone());
        assert_eq!(sequencer.gating_sequences.len(), 1);
        assert!(sequencer.remove_gating_sequence(gating_sequence.clone()));
        assert_eq!(sequencer.gating_sequences.len(), 0);
        assert!(!sequencer.remove_gating_sequence(gating_sequence.clone()));
    }

    #[test]
    fn test_create_sequence_barrier() {
        let sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        let barrier = sequencer.create_sequence_barrier();
        assert_eq!(barrier.get_cursor(), sequencer.get_cursor());
    }

    #[test]
    fn test_get_cursor() {
        let sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        assert_eq!(sequencer.get_cursor(), -1);
    }

    #[test]
    fn test_get_buffer_size() {
        let sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        assert_eq!(sequencer.get_buffer_size(), BUFFER_SIZE_I64);
    }

    #[test]
    fn test_has_available_capacity() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);

        sequencer.add_gating_sequences(Arc::new(AtomicSequence::default()));

        assert!(sequencer.has_available_capacity(1));
        assert!(sequencer.has_available_capacity(BUFFER_SIZE_I64));
        assert!(!sequencer.has_available_capacity(BUFFER_SIZE_I64 + 1));

        let optional_next = sequencer.next_one();

        if let Some(next) = optional_next {
            sequencer.publish(next, next);
            assert!(sequencer.has_available_capacity(BUFFER_SIZE_I64 - 1));
            assert!(!sequencer.has_available_capacity(BUFFER_SIZE_I64));
        } else {
            panic!("Expected a value but got None.");
        }
    }

    #[test]
    fn test_get_remaining_capacity() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        sequencer.add_gating_sequences(Arc::new(AtomicSequence::default()));
        assert_eq!(sequencer.get_remaining_capacity(), BUFFER_SIZE_I64);

        if let Some(next) = sequencer.next_one() {
            sequencer.publish(next, next);
            assert_eq!(sequencer.get_remaining_capacity(), BUFFER_SIZE_I64 - 1);
        } else {
            panic!("Expected a value but got None.");
        }
    }

    #[test]
    fn test_next() {
        let mut sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        sequencer.add_gating_sequences(Arc::new(AtomicSequence::default()));
        sequencer.claim(0);
        assert_eq!(sequencer.next(1), Some(1));
        assert_eq!(sequencer.next(BUFFER_SIZE_I64), None);
    }

    #[test]
    fn test_publish() {
        let sequencer = SingleProducerSequencer::new(BUFFER_SIZE, BusySpinWaitStrategy);
        sequencer.publish(0, 10);
        assert_eq!(sequencer.cursor.get(), 10);
    }
}
