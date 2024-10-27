use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::barrier::ProcessingSequenceBarrier;
use crate::sequence::{AtomicSequence, Sequence};
use crate::traits::Sequencer;
use crate::traits::WaitingStrategy;
use crate::utils::Utils;

pub struct SingleProducerSequencer<W: WaitingStrategy> {
    buffer_size: i64,
    cursor: Arc<AtomicSequence>,
    next_value: Sequence,
    cached_value: Sequence,
    gating_sequences: Vec<Arc<AtomicSequence>>,
    waiting_strategy: Arc<W>,
    is_done: Arc<AtomicBool>,
}

impl<W: WaitingStrategy> SingleProducerSequencer<W> {
    pub fn new(buffer_size: usize, waiting_strategy: W) -> Self {
        Self {
            buffer_size: buffer_size as i64,
            cursor: Arc::new(AtomicSequence::default()),
            next_value: Sequence::from(0),
            cached_value: Sequence::default(),
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
        let mut min_sequence = self.cached_value;
        let next = self.next_value;
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

        self.cached_value = min_sequence;
        self.next_value = end + 1;

        (start, end)
    }

    fn publish(&self, _: Sequence, high: Sequence) {
        self.cursor.set(high);
        self.waiting_strategy.signal_all_when_blocking();
    }

    fn drain(self) {
        let current = self.next_value - 1;
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
}
