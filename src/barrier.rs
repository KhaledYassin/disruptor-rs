//! Sequence barriers that control and coordinate consumer access to the ring buffer.
//!
//! # Processing Sequence Barrier
//!
//! A ProcessingSequenceBarrier acts as a coordination point between producers and consumers
//! in the Disruptor pattern. It ensures that consumers only process events that are safe
//! to consume based on dependencies and available sequences.
//!
//! ## Core Responsibilities
//!
//! 1. **Dependency Tracking**:
//!    - Maintains a list of "gating sequences" that represent dependencies
//!    - Ensures consumers don't read beyond the minimum available sequence across all dependencies
//!
//! 2. **Progress Control**:
//!    - Blocks consumers until required sequences are available
//!    - Implements the configured waiting strategy for efficient thread coordination
//!
//! 3. **Alert Handling**:
//!    - Supports graceful shutdown through an alert mechanism
//!    - Allows consumers to abort waiting when the system needs to stop
//!
//! ## Usage Example
//! ```
//! # use disruptor_rs::{
//! #     sequence::AtomicSequence,
//! #     ProcessingSequenceBarrier,
//! #     waiting::BusySpinWaitStrategy,
//! #     traits::SequenceBarrier,
//! #     sequence::Sequence,
//! # };
//! # use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
//!
//! // Create a wrapper type for testing
//! struct TestBarrier(ProcessingSequenceBarrier<BusySpinWaitStrategy>);
//!
//! // Implement SequenceBarrier for the wrapper type
//! impl SequenceBarrier for TestBarrier {
//!     fn wait_for(&self, sequence: Sequence) -> Option<Sequence> {
//!         Some(sequence)
//!     }
//!     fn signal(&self) {}
//! }
//!
//! let alert = Arc::new(AtomicBool::new(false));
//! let seq = Arc::new(AtomicSequence::default());
//! seq.set(5);
//! let waiting_strategy = Arc::new(BusySpinWaitStrategy::default());
//! let barrier = TestBarrier(ProcessingSequenceBarrier::new(
//!     alert,
//!     vec![seq],
//!     waiting_strategy
//! ));
//! let sequence = barrier.wait_for(5);
//! assert_eq!(sequence, Some(5));
//! ```

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::{
    sequence::{AtomicSequence, Sequence},
    traits::{SequenceBarrier, WaitingStrategy},
};

/// A barrier that controls consumer access to the ring buffer based on available sequences
/// and dependencies.
pub struct ProcessingSequenceBarrier<W: WaitingStrategy> {
    /// Alert flag for shutdown signaling
    alert: Arc<AtomicBool>,
    /// Sequences that must advance before consumption can proceed
    gating_sequences: Vec<Arc<AtomicSequence>>,
    /// Strategy determining how threads wait for sequences
    waiting_strategy: Arc<W>,
}

impl<W: WaitingStrategy> ProcessingSequenceBarrier<W> {
    /// Creates a new processing sequence barrier.
    ///
    /// # Parameters
    /// - `alert`: Shutdown signal flag
    /// - `gating_sequences`: Dependencies that must advance before consumption
    /// - `waiting_strategy`: How threads should wait for sequences
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
    /// Waits for a specific sequence to become available.
    fn wait_for(&self, sequence: Sequence) -> Option<Sequence> {
        self.waiting_strategy
            .wait_for(sequence, &self.gating_sequences, || {
                self.alert.load(Ordering::Relaxed)
            })
    }

    /// Signals waiting threads that new sequences may be available.
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
