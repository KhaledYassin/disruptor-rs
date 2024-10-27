use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::{
    sequence::{AtomicSequence, Sequence},
    traits::{SequenceBarrier, WaitingStrategy},
};

pub struct ProcessingSequenceBarrier<W: WaitingStrategy> {
    alert: Arc<AtomicBool>,
    gating_sequences: Vec<Arc<AtomicSequence>>,
    waiting_strategy: Arc<W>,
}

impl<W: WaitingStrategy> ProcessingSequenceBarrier<W> {
    pub fn new(
        alert: Arc<AtomicBool>,
        gating_sequences: Vec<Arc<AtomicSequence>>,
        waiting_strategy: Arc<W>,
    ) -> Self {
        Self {
            alert,
            gating_sequences,
            waiting_strategy,
        }
    }
}

impl<W: WaitingStrategy> SequenceBarrier for ProcessingSequenceBarrier<W> {
    fn wait_for(&self, sequence: Sequence) -> Option<Sequence> {
        self.waiting_strategy
            .wait_for(sequence, &self.gating_sequences, || {
                self.alert.load(Ordering::Relaxed)
            })
    }

    fn signal(&self) {
        self.waiting_strategy.signal_all_when_blocking()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waiting::BusySpinWaitStrategy;

    #[test]
    fn test_wait_for_returns_sequence_when_available() {
        let alert = Arc::new(AtomicBool::new(false));
        let seq = Arc::new(AtomicSequence::default());
        seq.set(5);
        let gating_sequences = vec![seq];
        let waiting_strategy = Arc::new(BusySpinWaitStrategy::new());

        let barrier = ProcessingSequenceBarrier::new(alert, gating_sequences, waiting_strategy);

        assert_eq!(barrier.wait_for(5), Some(5));
    }

    #[test]
    fn test_wait_for_returns_none_when_alerted() {
        let alert = Arc::new(AtomicBool::new(true));
        let seq = Arc::new(AtomicSequence::default());
        let gating_sequences = vec![seq];
        let waiting_strategy = Arc::new(BusySpinWaitStrategy::new());

        let barrier = ProcessingSequenceBarrier::new(alert, gating_sequences, waiting_strategy);

        assert_eq!(barrier.wait_for(5), None);
    }

    #[test]
    fn test_signal_calls_waiting_strategy() {
        let alert = Arc::new(AtomicBool::new(false));
        let seq = Arc::new(AtomicSequence::default());
        let gating_sequences = vec![seq];
        let waiting_strategy = Arc::new(BusySpinWaitStrategy::new());

        let barrier = ProcessingSequenceBarrier::new(alert, gating_sequences, waiting_strategy);

        barrier.signal(); // Just verify it doesn't panic
    }

    #[test]
    fn test_multiple_gating_sequences() {
        let alert = Arc::new(AtomicBool::new(false));
        let seq1 = Arc::new(AtomicSequence::default());
        let seq2 = Arc::new(AtomicSequence::default());
        seq1.set(5);
        seq2.set(3);
        let gating_sequences = vec![seq1, seq2];
        let waiting_strategy = Arc::new(BusySpinWaitStrategy::new());

        let barrier = ProcessingSequenceBarrier::new(alert, gating_sequences, waiting_strategy);

        // Should wait for minimum sequence (3)
        assert_eq!(barrier.wait_for(3), Some(3));
    }
}
