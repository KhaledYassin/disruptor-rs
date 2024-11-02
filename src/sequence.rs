//! Atomic sequence counters with cache line padding to prevent false sharing.
//!
//! # Cache Line Padding and False Sharing
//!
//! ## The Problem
//! Modern CPU architectures manage memory in cache lines (typically 64 bytes). When multiple
//! threads operate on variables that live in the same cache line but on different cores:
//!
//! 1. Any modification to a variable invalidates the entire cache line
//! 2. All cores must refresh their cache line copy, even if they only use unmodified variables
//! 3. This "false sharing" creates unnecessary cache coherence traffic and severely impacts performance
//!
//! ## The Solution
//! The `AtomicSequence` is carefully structured to occupy its own cache line by:
//! 1. Using explicit padding bytes to fill a complete cache line
//! 2. Ensuring 64-byte alignment through `#[repr(align(64))]`
//!
//! This isolation is crucial for the Disruptor's performance as sequences are frequently
//! accessed and modified by different threads:
//! - Publishers increment their sequences
//! - Consumers read sequences to track progress
//! - Gating sequences are monitored for backpressure
//!
//! Without padding, these concurrent operations would cause constant cache line invalidations
//! and significantly reduce throughput.

use std::sync::atomic::{AtomicI64, Ordering};

pub type Sequence = i64;

/// Size of a cache line on most modern CPUs (in bytes)
const CACHE_LINE_SIZE: usize = 64;
/// Padding bytes needed to fill a cache line after the atomic value
const CACHE_LINE_PADDING: usize = CACHE_LINE_SIZE - std::mem::size_of::<AtomicI64>();

/// An atomic sequence counter padded to occupy a full cache line.
///
/// # Memory Layout
/// ```text
/// |------------------------------------------|
/// |  AtomicI64 (8 bytes) | Padding (56 bytes)|
/// |------------------------------------------|
/// ^                                          ^
/// Cache line start                    Cache line end
/// ```
///
/// # Performance Characteristics
/// - Aligned to cache line boundaries (64 bytes)
/// - Isolated from other memory locations to prevent false sharing
/// - Atomic operations use appropriate memory ordering for consistency
///
/// # Usage in the Disruptor
/// Sequences are used throughout the Disruptor for various purposes:
/// - Tracking producer position in the ring buffer
/// - Maintaining consumer progress
/// - Implementing gating sequences for backpressure
/// - Coordinating multiple consumers in dependency graphs
#[repr(align(64))]
#[derive(Debug)]
pub struct AtomicSequence {
    /// The actual sequence value, atomically accessed
    value: AtomicI64,
    /// Padding to ensure the sequence occupies a full cache line
    /// This prevents false sharing with adjacent memory locations
    _padding: [u8; CACHE_LINE_PADDING],
}

impl AtomicSequence {
    // Create a new Sequence with an initial value.
    pub fn new(initial_value: Sequence) -> Self {
        AtomicSequence {
            value: AtomicI64::new(initial_value),
            _padding: [0u8; CACHE_LINE_PADDING],
        }
    }

    // Get the current value of the sequence.
    pub fn get(&self) -> Sequence {
        self.value.load(Ordering::Acquire)
    }

    // Set a new value for the sequence.
    pub fn set(&self, new_value: Sequence) {
        self.value.store(new_value, Ordering::Release);
    }

    // Increment the sequence by 1 and return the new value.
    pub fn get_and_increment(&self) -> Sequence {
        self.value.fetch_add(1, Ordering::SeqCst)
    }

    // Increment the sequence by 1 and return the old value.
    pub fn increment_and_get(&self) -> Sequence {
        self.value.fetch_add(1, Ordering::SeqCst) + 1
    }
}

impl Default for AtomicSequence {
    fn default() -> Self {
        Self::new(-1)
    }
}

impl From<i64> for AtomicSequence {
    fn from(value: i64) -> Self {
        Self::new(value)
    }
}

impl PartialEq for AtomicSequence {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn test_sequence() {
        let sequence = AtomicSequence::new(0);
        assert_eq!(sequence.get(), 0);
        assert_eq!(sequence.get_and_increment(), 0);
        assert_eq!(sequence.get(), 1);
        assert_eq!(sequence.increment_and_get(), 2);
        assert_eq!(sequence.get(), 2);
        sequence.set(42);
        assert_eq!(sequence.get(), 42);
    }

    #[test]
    fn test_sequence_concurrent() {
        let sequence = AtomicSequence::new(0);
        let mut handles = vec![];
        let sequence_arc = Arc::new(sequence);
        for _ in 0..10 {
            let sequence = sequence_arc.clone();
            let handle = std::thread::spawn(move || {
                for _ in 0..1000 {
                    sequence.get_and_increment();
                }
            });
            handles.push(handle);
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(sequence_arc.get(), 10000);
    }

    #[test]
    fn test_sequence_concurrent_increment_and_get() {
        let sequence = AtomicSequence::new(0);
        let mut handles = vec![];
        let sequence_arc = Arc::new(sequence);
        for _ in 0..10 {
            let sequence = sequence_arc.clone();
            let handle = std::thread::spawn(move || {
                for _ in 0..1000 {
                    sequence.increment_and_get();
                }
            });
            handles.push(handle);
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(sequence_arc.get(), 10000);
    }
}
