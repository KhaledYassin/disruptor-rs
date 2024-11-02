//! Waiting strategies for coordinating producers and consumers in the Disruptor pattern.
//!
//! # Purpose of Waiting Strategies
//!
//! Waiting strategies manage how threads behave when they cannot make immediate progress:
//! - Producers wait when the ring buffer is full (backpressure)
//! - Consumers wait when no new items are available
//!
//! The choice of waiting strategy represents a trade-off between:
//! - Latency (how quickly threads respond to new data)
//! - CPU usage (how efficiently threads wait)
//! - Thread context switching
//!
//! # Available Strategies
//!
//! ## BusySpinWaitStrategy
//! ```rust
//! /// Highest performance waiting strategy that continuously polls for updates.
//! ///
//! /// # Characteristics
//! /// - Lowest latency (sub-microsecond response times)
//! /// - Highest CPU usage (100% core utilization)
//! /// - No context switching
//! ///
//! /// # Best Used When
//! /// - Latency is critical
//! /// - Dedicated CPU cores are available
//! /// - Running on systems with consistent processing time requirements
//! #[derive(Default)]
//! pub struct BusySpinWaitStrategy;
//! ```
//!
//! ## YieldingWaitStrategy
//! ```rust
//! /// Balanced waiting strategy that spins for a while before yielding.
//! ///
//! /// # Characteristics
//! /// - Medium-low latency (few microseconds)
//! /// - Moderate CPU usage
//! /// - Occasional context switching
//! ///
//! /// # Implementation Details
//! /// 1. Spins for 100 iterations checking for updates
//! /// 2. Calls `thread::yield_now()` to allow other threads to run
//! ///
//! /// # Best Used When
//! /// - Balance between latency and CPU usage is needed
//! /// - System is under mixed workload
//! #[derive(Default)]
//! pub struct YieldingWaitStrategy;
//! ```
//!
//! ## SleepingWaitStrategy
//! ```rust
//! /// Conservative waiting strategy that sleeps between checks.
//! ///
//! /// # Characteristics
//! /// - Higher latency (millisecond range)
//! /// - Lowest CPU usage
//! /// - Regular context switching
//! ///
//! /// # Implementation Details
//! /// 1. Spins for 200 iterations checking for updates
//! /// 2. Sleeps for 1 microsecond between checks
//! ///
//! /// # Best Used When
//! /// - CPU efficiency is more important than latency
//! /// - Processing times are variable or unpredictable
//! /// - Running on systems with power/thermal constraints
//! #[derive(Default)]
//! pub struct SleepingWaitStrategy;
//! ```
//!
//! # Choosing a Strategy
//!
//! Selection criteria should consider:
//! 1. Latency requirements
//! 2. Available CPU resources
//! 3. System power constraints
//! 4. Other workloads on the system
//!
//! ## Performance Impact
//! The waiting strategy can significantly impact the Disruptor's performance:
//! - BusySpinWaitStrategy: Nanosecond response times but high CPU usage
//! - YieldingWaitStrategy: Microsecond response times with moderate CPU usage
//! - SleepingWaitStrategy: Millisecond response times but minimal CPU impact
//!
//! ## System Considerations
//! - Number of available cores
//! - Power consumption requirements
//! - Operating system scheduling policies
//! - Other applications running on the system

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
