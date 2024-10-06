use std::sync::Arc;

use crate::sequence::{AtomicSequence, Sequence};

// A trait for providing a sequence barrier.
// # Types
// - `Sequence`: The type of sequence used.
// - `AtomicSequence`: The type of atomic sequence used.
// # Methods
// - `get_cursor`: Returns the current cursor value.
// - `wait_for`: Waits for the given sequence to be available.
// - `is_alerted`: Returns true if the barrier has been alerted.
// - `alert`: Alerts the barrier.
// - `clear_alert`: Clears the alert.
pub trait SequenceBarrier: Send + Sync {
    fn get_cursor(&self) -> Sequence;
    fn wait_for(&self, sequence: Sequence) -> Option<Sequence>;
    fn is_alerted(&self) -> bool;
    fn alert(&self);
    fn clear_alert(&self);
}

// A trait for providing a sequencer.
// # Types
// - `Barrier`: The type of sequence barrier used.
// # Methods
// - `claim`: Claims the given sequence.
// - `is_available`: Returns true if the given sequence is available.
// - `add_gating_sequences`: Adds the given gating sequences.
// - `remove_gating_sequence`: Removes the given gating sequence.
// - `create_sequence_barrier`: Creates a new sequence barrier.
// - `get_cursor`: Returns the current cursor value.
// - `get_buffer_size`: Returns the buffer size.
// - `has_available_capacity`: Returns true if the buffer has available capacity.
// - `get_remaining_capacity`: Returns the remaining capacity.
// - `next_one`: Returns the next sequence.
// - `next`: Returns the next `n` sequences.
// - `publish`: Publishes the given sequences.
// - `drain`: Drains the sequencer.
pub trait Sequencer {
    type Barrier: SequenceBarrier;
    // Inteferface methods
    fn claim(&mut self, sequence: Sequence);
    fn is_available(&self, sequence: Sequence) -> bool;
    fn add_gating_sequence(&mut self, gating_sequence: Arc<AtomicSequence>);
    fn remove_gating_sequence(&mut self, sequence: Arc<AtomicSequence>) -> bool;
    fn create_sequence_barrier(&self, gating_sequences: &[Arc<AtomicSequence>]) -> Self::Barrier;

    // Abstract methods
    fn get_cursor(&self) -> Arc<AtomicSequence>;
    fn get_buffer_size(&self) -> i64;
    fn has_available_capacity(&mut self, required_capacity: Sequence) -> bool;
    fn get_remaining_capacity(&self) -> Sequence;
    fn next_one(&mut self) -> Option<(Sequence, Sequence)> {
        self.next(1)
    }
    fn next(&mut self, n: Sequence) -> Option<(Sequence, Sequence)>;
    fn publish(&self, low: Sequence, high: Sequence);
    fn drain(self);
}

/// A trait for providing a waiting strategy.
/// # Methods
/// - `wait_for`: Waits for the given sequence to be available.
/// - `signal_all_when_blocking`: Signals all when blocking.
pub trait WaitingStrategy: Default + Send + Sync {
    fn wait_for<F: Fn() -> bool>(
        &self,
        sequence: Sequence,
        dependencies: &[Arc<AtomicSequence>],
        check_alert: F,
    ) -> Option<i64>;

    fn signal_all_when_blocking(&self);
}

/// A trait for providing data from a buffer.
/// # Types
/// - `T`: The type of elements in the buffer.
/// # Safety
/// This trait is unsafe because it allows for mutable access to the buffer. It is up to the implementor
/// to ensure that the buffer is accessed correctly.
///
/// # Methods
/// - `get_capacity`: Returns the capacity of the buffer.
/// - `get`: Returns a reference to the element at the given sequence.
/// - `get_mut`: Returns a mutable reference to the element at the given sequence.
#[allow(clippy::mut_from_ref)]
pub trait DataProvider<T>: Send + Sync {
    fn get_capacity(&self) -> usize;
    unsafe fn get(&self, sequence: Sequence) -> &T;
    unsafe fn get_mut(&self, sequence: Sequence) -> &mut T;
}

/// A trait for providing a runnable object.
/// # Methods
/// - `start`: Starts the runnable object.
/// - `stop`: Stops the runnable object.
/// - `run`: Runs the runnable object.
/// - `is_running`: Returns true if the runnable object is running.
pub trait Runnable: Send {
    fn run(&self);
    fn stop(&mut self);
    fn is_running(&self) -> bool;
}

/// A trait for providing an event processor.
/// # Types
/// - `T`: The type of events to process.
/// # Methods
/// - `create`: Creates a new event processor.
/// - `get_sequence`: Returns the sequence of the event processor.
pub trait EventProcessor<'a, T> {
    fn get_cursor(&self) -> Arc<AtomicSequence>;
    fn create<D: DataProvider<T> + 'a, S: SequenceBarrier + 'a>(
        self,
        data_provider: Arc<D>,
        barrier: S,
    ) -> Box<dyn Runnable + 'a>;
    fn get_sequence(&self) -> Arc<AtomicSequence>;
}

/// A trait for providing an event handler.
/// # Types
/// - `T`: The type of events to handle.
/// # Methods
/// - `on_event`: Handles the given event.
/// - `on_start`: Called when the event handler starts.
/// - `on_shutdown`: Called when the event handler shuts down.
pub trait EventHandler<T> {
    fn on_event(&self, event: &T, sequence: Sequence, end_of_batch: bool);
    fn on_start(&self);
    fn on_shutdown(&self);
}

/// A trait for providing an executor thread handle.
/// # Methods
/// - `join`: Joins the executor thread.
pub trait ExecutorHandle {
    fn join(self);
}

/// A trait for providing an executor.
/// # Types
/// - `Handle`: The type of executor handle.
/// # Methods
/// - `with_runnales`: Creates a new executor with the given runnables.
/// - `spwan`: Spawns the executor.
pub trait EventProcessorExecutor<'a> {
    type Handle: ExecutorHandle;
    fn with_runnables(runnables: Vec<Box<dyn Runnable + 'a>>) -> Self;
    fn spawn(self) -> Self::Handle;
}

/// A trait for producing events.
/// # Types
/// - `Item`: The type of events to produce.
/// # Methods
/// - `write`: Writes the given event.
/// - `drain`: Drains the event producer.
pub trait EventProducer<'a> {
    type Item;

    fn write<F, U, I, E>(&mut self, items: I, f: F)
    where
        I: IntoIterator<Item = U, IntoIter = E>,
        E: ExactSizeIterator<Item = U>,
        F: Fn(&mut Self::Item, Sequence, &U);

    fn drain(self);
}
