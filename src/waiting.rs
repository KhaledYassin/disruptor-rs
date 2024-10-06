use std::sync::Arc;

use crate::{
    sequence::{AtomicSequence, Sequence},
    traits::WaitingStrategy,
    utils::Utils,
};

#[derive(Default)]
pub struct BusySpinWaitStrategy;

impl WaitingStrategy for BusySpinWaitStrategy {
    fn wait_for<F: Fn() -> bool>(
        &self,
        sequence: Sequence,
        dependencies: &[Arc<AtomicSequence>],
        check_alert: F,
    ) -> Option<i64> {
        loop {
            let minimum_sequence = Utils::get_minimum_sequence(dependencies, sequence);

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
    fn wait_for<F: Fn() -> bool>(
        &self,
        sequence: Sequence,
        dependencies: &[Arc<AtomicSequence>],
        check_alert: F,
    ) -> Option<i64> {
        let mut counter = 100;

        loop {
            let minimum_sequence = Utils::get_minimum_sequence(dependencies, sequence);

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
    fn wait_for<F: Fn() -> bool>(
        &self,
        sequence: Sequence,
        dependencies: &[Arc<AtomicSequence>],
        check_alert: F,
    ) -> Option<i64> {
        let mut counter = 200;

        loop {
            let minimum_sequence = Utils::get_minimum_sequence(dependencies, sequence);

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
