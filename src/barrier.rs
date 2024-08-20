use std::sync::{atomic::AtomicBool, Arc};

use crate::{
    sequence::{AtomicSequence, Sequence},
    utils::Utils,
    waiting::WaitingStrategy,
};

pub trait SequenceBarrier: Send + Sync {
    fn get_cursor(&self) -> Sequence;
    fn wait_for(
        &self,
        sequence: &AtomicSequence,
        dependencies: &[Arc<AtomicSequence>],
    ) -> Option<Sequence>;
    fn is_alerted(&self) -> bool;
    fn alert(&self);
    fn clear_alert(&self);
}

pub struct ProcessingSequenceBarrier<W: WaitingStrategy> {
    cursor: Arc<AtomicSequence>,
    alert: Arc<AtomicBool>,
    dependencies: Vec<Arc<AtomicSequence>>,
    waiting_strategy: Arc<W>,
}

impl<W: WaitingStrategy> ProcessingSequenceBarrier<W> {
    pub fn new(
        cursor: Arc<AtomicSequence>,
        alert: Arc<AtomicBool>,
        dependencies: Vec<Arc<AtomicSequence>>,
        waiting_strategy: Arc<W>,
    ) -> Self {
        Self {
            cursor,
            alert,
            dependencies,
            waiting_strategy,
        }
    }
}

impl<W: WaitingStrategy> SequenceBarrier for ProcessingSequenceBarrier<W> {
    fn get_cursor(&self) -> Sequence {
        self.cursor.get()
    }

    fn wait_for(
        &self,
        sequence: &AtomicSequence,
        dependencies: &[Arc<AtomicSequence>],
    ) -> Option<Sequence> {
        let optional_available_sequence =
            self.waiting_strategy
                .wait_for(sequence, dependencies, || self.is_alerted());

        match optional_available_sequence {
            Some(available_sequence) => {
                if available_sequence < sequence.get() {
                    Some(available_sequence)
                } else {
                    Some(Utils::get_maximum_sequence(&self.dependencies))
                }
            }
            None => None,
        }
    }

    fn is_alerted(&self) -> bool {
        self.alert.load(std::sync::atomic::Ordering::Acquire)
    }

    fn alert(&self) {
        self.alert.store(true, std::sync::atomic::Ordering::Release);
        self.waiting_strategy.signal_all_when_blocking();
    }

    fn clear_alert(&self) {
        self.alert
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;

    #[derive(Default)]
    struct DummyWaitingStrategy;

    impl WaitingStrategy for DummyWaitingStrategy {
        fn wait_for<F: Fn() -> bool>(
            &self,
            _sequence: &AtomicSequence,
            _dependencies: &[Arc<AtomicSequence>],
            _is_alerted: F,
        ) -> Option<Sequence> {
            let _ = _is_alerted;
            None
        }

        fn signal_all_when_blocking(&self) {}
    }

    #[test]
    fn test_sequence_barrier() {
        let cursor = Arc::new(AtomicSequence::new(0));
        let alert = Arc::new(AtomicBool::new(false));
        let dependency1 = Arc::new(AtomicSequence::new(0));
        let dependency2 = Arc::new(AtomicSequence::new(0));
        let dependencies = vec![dependency1.clone(), dependency2.clone()];
        let waiting_strategy = Arc::new(DummyWaitingStrategy);

        let barrier = ProcessingSequenceBarrier::new(
            cursor.clone(),
            alert.clone(),
            dependencies.clone(),
            waiting_strategy.clone(),
        );

        // Test get_cursor
        assert_eq!(barrier.get_cursor(), 0);

        // Test wait_for
        let sequence = AtomicSequence::new(5);
        let result = barrier.wait_for(&sequence, &dependencies);
        assert_eq!(result, None);

        // Test is_alerted
        assert!(!barrier.is_alerted());

        // Test alert
        barrier.alert();
        assert!(alert.load(Ordering::Acquire));

        // Test clear_alert
        barrier.clear_alert();
        assert!(!alert.load(Ordering::Acquire));
    }
}
