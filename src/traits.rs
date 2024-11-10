//! Core traits defining the Disruptor pattern interface.
//!
//! This module contains the fundamental traits that make up the Disruptor pattern:
//! - Event processing and handling
//! - Sequence management
//! - Data access and storage
//! - Execution control
//!
//! # Core Traits Overview
//!
//! ## Event Processing
//! - [`EventProcessor`]: Processes events from the ring buffer
//! - [`EventHandler`]: Handles individual events
//! - [`EventProducer`]: Produces events into the ring buffer
//!
//! ## Sequence Management
//! - [`Sequencer`]: Manages sequences for event coordination
//! - [`SequenceBarrier`]: Controls access to sequences
//! - [`WaitingStrategy`]: Defines how threads wait for available sequences
//!
//! ## Data Access
//! - [`DataProvider`]: Provides safe and unsafe access to the underlying buffer
//!
//! ## Execution
//! - [`Runnable`]: Base interface for executable components
//! - [`EventProcessorExecutor`]: Manages execution of event processors
//! - [`ExecutorHandle`]: Controls executor lifecycle
//!
//! # Examples
//!
//! ```rust
//! use disruptor_rs::{EventHandler, sequence::Sequence};
//!
//! struct MyHandler;
//! impl EventHandler<i64> for MyHandler {
//!     fn on_event(&self, event: &i64, sequence: Sequence, end_of_batch: bool) {
//!         // Process the event
//!     }
//!     fn on_start(&self) {}
//!     fn on_shutdown(&self) {}
//! }
//! ```

use std::sync::Arc;

use crate::sequence::{AtomicSequence, Sequence};

/// Controls access to sequences in the ring buffer.
///
/// A sequence barrier determines when sequences are available for processing
/// and manages coordination between different components.
///
/// # Methods
/// * `wait_for` - Blocks until a sequence becomes available
/// * `signal` - Signals that new sequences may be available
pub trait SequenceBarrier: Send + Sync {
    fn wait_for(&self, sequence: Sequence) -> Option<Sequence>;
    fn signal(&self);
}

/// Manages sequence generation and coordination in the ring buffer.
///
/// The sequencer is responsible for generating new sequence numbers and
/// managing the relationship between publishers and subscribers.
///
/// # Type Parameters
/// * `Barrier` - The type of sequence barrier used by this sequencer
///
/// # Methods
/// * `add_gating_sequence` - Adds a gating sequence to the sequencer
/// * `remove_gating_sequence` - Removes a gating sequence from the sequencer
/// * `create_sequence_barrier` - Creates a sequence barrier with the given gating sequences
/// * `get_cursor` - Returns the current cursor value
/// * `next` - Gets the next sequence value
/// * `publish` - Publishes the given sequence range
/// * `drain` - Drains the sequencer
pub trait Sequencer {
    type Barrier: SequenceBarrier;
    // Inteferface methods
    fn add_gating_sequence(&mut self, gating_sequence: &Arc<AtomicSequence>);
    fn remove_gating_sequence(&mut self, sequence: &Arc<AtomicSequence>) -> bool;
    fn create_sequence_barrier(&self, gating_sequences: &[Arc<AtomicSequence>]) -> Self::Barrier;

    // Abstract methods
    fn get_cursor(&self) -> Arc<AtomicSequence>;
    fn next(&self, n: Sequence) -> (Sequence, Sequence);
    fn publish(&self, low: Sequence, high: Sequence);
    fn drain(self);
}

/// Defines how threads wait for available sequences.
///
/// Implements the strategy pattern for different waiting behaviors when
/// sequences are not yet available.
///
/// # Examples
///
/// ```
/// use disruptor_rs::traits::WaitingStrategy;
/// use disruptor_rs::sequence::AtomicSequence;
///
/// #[derive(Default)]
/// struct BlockingWaitStrategy;
///
/// impl WaitingStrategy for BlockingWaitStrategy {
///     fn new() -> Self {
///         BlockingWaitStrategy
///     }
///
///     fn wait_for<F: Fn() -> bool>(
///         &self,
///         sequence: i64,
///         dependencies: &[std::sync::Arc<AtomicSequence>],
///         check_alert: F
///     ) -> Option<i64> {
///         // Implementation would go here
///         None
///     }
///
///     fn signal_all_when_blocking(&self) {
///         // Implementation would go here
///     }
/// }
/// ```
///
/// # Methods
/// * `new` - Creates a new instance of the waiting strategy
/// * `wait_for` - Waits for the given sequence to be available
/// * `signal_all_when_blocking` - Signals that all threads should be blocked
pub trait WaitingStrategy: Default + Send + Sync {
    fn new() -> Self;
    fn wait_for<F: Fn() -> bool>(
        &self,
        sequence: Sequence,
        dependencies: &[Arc<AtomicSequence>],
        check_alert: F,
    ) -> Option<i64>;

    fn signal_all_when_blocking(&self);
}

/// Provides safe and unsafe access to the underlying ring buffer.
///
/// # Safety
///
/// This trait provides both safe and unsafe methods for accessing the buffer.
/// Implementors must ensure proper bounds checking and memory safety.
///
/// # Type Parameters
/// * `T` - The type of elements stored in the buffer.
///
/// This lint allow is necessary because DataProvider::get_mut returns a mutable reference from an immutable one.
/// This is safe in our case because we use interior mutability (UnsafeCell) in the RingBuffer implementation,
/// and the safety is guaranteed by the Sequencer which ensures proper synchronization of access to the buffer.
#[allow(clippy::mut_from_ref)]
pub trait DataProvider<T>: Send + Sync {
    fn get_capacity(&self) -> usize;

    /// # Safety
    /// Caller must ensure the sequence is valid and in bounds
    unsafe fn get(&self, sequence: Sequence) -> &T;

    /// # Safety
    /// Caller must ensure the sequence is valid and in bounds and no other references exist
    unsafe fn get_mut(&self, sequence: Sequence) -> &mut T;
}

/// Defines the lifecycle operations for executable components.
///
/// Provides basic control operations for starting, stopping, and
/// checking the status of running components.
///
/// # Methods
/// * `run` - Starts the component
/// * `stop` - Stops the component
/// * `is_running` - Checks if the component is running
pub trait Runnable: Send {
    fn run(&mut self);
    fn stop(&mut self);
    fn is_running(&self) -> bool;
}

/// A trait for providing an event processor.
/// # Types
/// - `T`: The type of events to process.
/// # Methods
/// * `create`: Creates a new event processor.
/// * `get_sequence`: Returns the sequence of the event processor.
pub trait EventProcessor<'a, T> {
    fn get_cursor(&self) -> Arc<AtomicSequence>;
    fn create<D: DataProvider<T> + 'a, S: SequenceBarrier + 'a>(
        self,
        data_provider: Arc<D>,
        barrier: S,
    ) -> Box<dyn Runnable + 'a>;
    fn get_sequence(&self) -> Arc<AtomicSequence>;
}

/// A trait for providing an event processor with mutable access to the event.
pub trait EventProcessorMut<'a, T> {
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
/// * `on_event`: Handles the given event.
/// * `on_start`: Called when the event handler starts.
/// * `on_shutdown`: Called when the event handler shuts down.
pub trait EventHandler<T> {
    fn on_event(&self, event: &T, sequence: Sequence, end_of_batch: bool);
    fn on_start(&self);
    fn on_shutdown(&self);
}

/// A trait for providing an event handler with mutable access to the event.
/// # Types
/// - `T`: The type of events to handle.
/// # Methods
/// * `on_event`: Handles the given event.
/// * `on_start`: Called when the event handler starts.
/// * `on_shutdown`: Called when the event handler shuts down.
pub trait EventHandlerMut<T> {
    fn on_event(&mut self, event: &T, sequence: Sequence, end_of_batch: bool);
    fn on_start(&mut self);
    fn on_shutdown(&mut self);
}

/// A trait for providing an executor thread handle.
/// # Methods
/// * `join`: Joins the executor thread.
pub trait ExecutorHandle {
    fn join(self);
}

/// A trait for providing an executor.
/// # Types
/// - `Handle`: The type of executor handle.
/// # Methods
/// * `with_runnales`: Creates a new executor with the given runnables.
/// * `spwan`: Spawns the executor.
pub trait EventProcessorExecutor<'a> {
    type Handle: ExecutorHandle;
    fn with_runnables(runnables: Vec<Box<dyn Runnable + 'a>>) -> Self;
    fn spawn(self) -> Self::Handle;
}

/// A trait for producing events.
/// # Types
/// - `Item`: The type of events to produce.
/// # Methods
/// * `write`: Writes the given event.
/// * `drain`: Drains the event producer.
pub trait EventProducer<'a> {
    type Item;

    fn write<F, U, I, E>(&self, items: I, f: F)
    where
        I: IntoIterator<Item = U, IntoIter = E>,
        E: ExactSizeIterator<Item = U>,
        F: Fn(&mut Self::Item, Sequence, &U);

    fn drain(self);
}
