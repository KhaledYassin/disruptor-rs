use std::sync::Arc;

use crate::{
    sequence::{AtomicSequence, Sequence},
    traits::WaitingStrategy,
    utils::Utils,
};

#[derive(Default)]
pub struct BusySpinWaitStrategy;

impl WaitingStrategy for BusySpinWaitStrategy {
    fn new() -> Self {
        BusySpinWaitStrategy {}
    }

    fn wait_for<F: Fn() -> bool>(
        &self,
        sequence: Sequence,
        dependencies: &[Arc<AtomicSequence>],
        check_alert: F,
    ) -> Option<i64> {
        loop {
            let minimum_sequence = Utils::get_minimum_sequence(dependencies);

            if minimum_sequence >= sequence {
                return Some(minimum_sequence);
            }

            if check_alert() {
                return None;
            }
        }
    }

    fn signal_all_when_blocking(&self) {}
}

#[derive(Default)]
pub struct YieldingWaitStrategy;

impl WaitingStrategy for YieldingWaitStrategy {
    fn new() -> Self {
        YieldingWaitStrategy {}
    }
    fn wait_for<F: Fn() -> bool>(
        &self,
        sequence: Sequence,
        dependencies: &[Arc<AtomicSequence>],
        check_alert: F,
    ) -> Option<i64> {
        let mut counter = 100;
        loop {
            let minimum_sequence = Utils::get_minimum_sequence(dependencies);

            if minimum_sequence >= sequence {
                return Some(minimum_sequence);
            }

            if check_alert() {
                return None;
            }

            counter -= 1;

            if counter <= 0 {
                std::thread::yield_now();
                counter = 100;
            }
        }
    }

    fn signal_all_when_blocking(&self) {}
}

#[derive(Default)]
pub struct SleepingWaitStrategy;

impl WaitingStrategy for SleepingWaitStrategy {
    fn new() -> Self {
        SleepingWaitStrategy {}
    }

    fn wait_for<F: Fn() -> bool>(
        &self,
        sequence: Sequence,
        dependencies: &[Arc<AtomicSequence>],
        check_alert: F,
    ) -> Option<i64> {
        let mut counter = 200;
        loop {
            let minimum_sequence = Utils::get_minimum_sequence(dependencies);

            if minimum_sequence >= sequence {
                return Some(minimum_sequence);
            }

            if check_alert() {
                return None;
            }

            counter -= 1;

            if counter <= 0 {
                std::thread::sleep(std::time::Duration::from_micros(1));
                counter = 200;
            }
        }
    }

    fn signal_all_when_blocking(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_busy_spin_wait_strategy() {
        let strategy = BusySpinWaitStrategy::new();
        let seq = Arc::new(AtomicSequence::default());
        let dependencies = vec![seq.clone()];

        // Test returns None when alerted
        assert_eq!(strategy.wait_for(1, &dependencies, || true), None);

        // Test returns sequence when available
        seq.set(5);
        assert_eq!(strategy.wait_for(5, &dependencies, || false), Some(5));

        // Test waits for sequence
        let seq_clone = seq.clone();
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            seq_clone.set(10);
        });

        assert_eq!(strategy.wait_for(10, &dependencies, || false), Some(10));
        handle.join().unwrap();
    }

    #[test]
    fn test_sleeping_wait_strategy() {
        let strategy = SleepingWaitStrategy::new();
        let seq = Arc::new(AtomicSequence::default());
        let dependencies = vec![seq.clone()];

        // Test returns None when alerted
        assert_eq!(strategy.wait_for(1, &dependencies, || true), None);

        // Test returns sequence when available
        seq.set(5);
        assert_eq!(strategy.wait_for(5, &dependencies, || false), Some(5));

        // Test waits for sequence
        let seq_clone = seq.clone();
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            seq_clone.set(10);
        });

        assert_eq!(strategy.wait_for(10, &dependencies, || false), Some(10));
        handle.join().unwrap();
    }

    #[test]
    fn test_multiple_dependencies() {
        let strategy = BusySpinWaitStrategy::new();
        let seq1 = Arc::new(AtomicSequence::default());
        let seq2 = Arc::new(AtomicSequence::default());
        let dependencies = vec![seq1.clone(), seq2.clone()];

        seq1.set(5);
        seq2.set(3);

        // Should wait for minimum sequence (3)
        assert_eq!(strategy.wait_for(3, &dependencies, || false), Some(3));
    }
}
