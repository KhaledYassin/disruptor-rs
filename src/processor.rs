//! Event processors that consume and handle events from the ring buffer.
//!
//! # Understanding Processors
//!
//! Processors are the consumers in the Disruptor pattern. They:
//! 1. Read events from specific sequences in the ring buffer
//! 2. Process events through user-defined handlers
//! 3. Track their progress using sequence counters
//!
//! # Usage Examples
//!
//! ## Basic Single Consumer
//! ```rust
//! use my_disruptor::{
//!     EventProcessorFactory,
//!     RingBuffer,
//!     EventHandler,
//!     WaitStrategy,
//!     Sequence,
//! };
//!
//! // 1. Define your event handler
//! struct MyHandler;
//! impl EventHandler<MyEvent> for MyHandler {
//!     fn on_event(&self, event: &MyEvent, sequence: Sequence, end_of_batch: bool) {
//!         // Process the event
//!         println!("Processing event at sequence {}: {:?}", sequence, event);
//!     }
//!
//!     // Optional lifecycle methods
//!     fn on_start(&self) {
//!         println!("Handler started");
//!     }
//!
//!     fn on_shutdown(&self) {
//!         println!("Handler shutting down");
//!     }
//! }
//!
//! // 2. Create and configure the processor
//! let handler = MyHandler;
//! let processor = EventProcessorFactory::create(handler);
//!
//! // 3. Connect to ring buffer
//! let runnable = processor.create(
//!     ring_buffer.clone(),
//!     barrier
//! );
//!
//! // 4. Run the processor (typically in its own thread)
//! std::thread::spawn(move || {
//!     runnable.run();
//! });
//! ```
//!
//! ## Multiple Dependent Consumers
//! ```rust
//! // Create processors with dependencies
//! let processor_a = EventProcessorFactory::create(HandlerA);
//! let processor_b = EventProcessorFactory::create(HandlerB);
//!
//! // Get processor A's sequence for processor B's barrier
//! let seq_a = processor_a.get_sequence();
//!
//! // Create barriers with dependencies
//! let barrier_a = ring_buffer.new_barrier();
//! let barrier_b = ring_buffer.new_barrier_with_sequences(vec![seq_a]);
//!
//! // Create runnables
//! let runnable_a = processor_a.create(ring_buffer.clone(), barrier_a);
//! let runnable_b = processor_b.create(ring_buffer.clone(), barrier_b);
//!
//! // Run processors
//! std::thread::spawn(move || runnable_a.run());
//! std::thread::spawn(move || runnable_b.run());
//! ```
//!
//! # Best Practices
//!
//! 1. **Handler Design**:
//!    - Keep handlers stateless if possible
//!    - Minimize processing time in `on_event`
//!    - Use `end_of_batch` for batch optimizations
//!
//! 2. **Dependencies**:
//!    - Create clear processing chains
//!    - Avoid circular dependencies
//!    - Consider using multiple ring buffers for complex flows
//!
//! 3. **Performance**:
//!    - Choose appropriate wait strategies
//!    - Monitor sequence progress
//!    - Consider batch sizes in handler logic
//!
//! # Error Handling
//!
//! Handlers should manage their own error handling:
//! ```rust
//! impl EventHandler<MyEvent> for MyHandler {
//!     fn on_event(&self, event: &MyEvent, sequence: Sequence, end_of_batch: bool) {
//!         match process_event(event) {
//!             Ok(_) => {
//!                 // Normal processing
//!             }
//!             Err(e) => {
//!                 // Log error but continue processing
//!                 log::error!("Error processing event at {}: {:?}", sequence, e);
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! # Shutdown Handling
//!
//! Proper shutdown sequence:
//! ```rust
//! // Signal shutdown
//! runnable.stop();
//!
//! // Wait for processing to complete
//! while runnable.is_running() {
//!     std::thread::sleep(std::time::Duration::from_millis(100));
//! }
//! ```

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use crate::{
    sequence::AtomicSequence,
    traits::{DataProvider, EventHandler, EventProcessor, Runnable, SequenceBarrier},
};

pub struct EventProcessorFactory;

impl EventProcessorFactory {
    pub fn create<'a, E, T>(handler: E) -> impl EventProcessor<'a, T>
    where
        E: EventHandler<T> + Send + 'a,
        T: Send + 'a,
    {
        Processor {
            handler,
            cursor: Default::default(),
            _marker: Default::default(),
        }
    }
}

struct Processor<E, T> {
    handler: E,
    cursor: Arc<AtomicSequence>,
    _marker: std::marker::PhantomData<T>,
}

enum RunnableProcessorState {
    Idle = 0,
    Halted = 1,
    Running = 2,
}

struct RunnableProcessor<E, T, D: DataProvider<T>, B: SequenceBarrier> {
    running: AtomicU8,
    processor: Processor<E, T>,
    data_provider: Arc<D>,
    barrier: B,
}

impl<E, T, D, B> RunnableProcessor<E, T, D, B>
where
    E: EventHandler<T> + Send,
    D: DataProvider<T>,
    B: SequenceBarrier,
    T: Send,
{
    fn process_events(&self) {
        let f = &self.processor.handler;
        let cursor = &self.processor.cursor;
        let data_provider = &self.data_provider;
        let barrier = &self.barrier;

        while self.running.load(Ordering::Acquire) == RunnableProcessorState::Running as u8 {
            let next_sequence = cursor.get() + 1;
            let available_sequence = barrier.wait_for(next_sequence);

            match available_sequence {
                Some(available_sequence) => {
                    for i in next_sequence..=available_sequence {
                        let event = unsafe { data_provider.get(i) };
                        f.on_event(event, i, i == available_sequence);
                    }

                    cursor.set(available_sequence);
                    barrier.signal();
                }
                None => {
                    return;
                }
            }
        }
    }
}

impl<'a, E, T> EventProcessor<'a, T> for Processor<E, T>
where
    E: EventHandler<T> + Send + 'a,
    T: Send + 'a,
{
    fn get_cursor(&self) -> Arc<AtomicSequence> {
        self.cursor.clone()
    }

    fn create<D: DataProvider<T> + 'a, S: SequenceBarrier + 'a>(
        self,
        data_provider: Arc<D>,
        barrier: S,
    ) -> Box<dyn Runnable + 'a> {
        Box::new(RunnableProcessor {
            running: AtomicU8::new(RunnableProcessorState::Idle as u8),
            processor: self,
            data_provider,
            barrier,
        })
    }

    fn get_sequence(&self) -> Arc<AtomicSequence> {
        self.cursor.clone()
    }
}

impl<E, T, D, B> Runnable for RunnableProcessor<E, T, D, B>
where
    E: EventHandler<T> + Send,
    D: DataProvider<T>,
    B: SequenceBarrier,
    T: Send,
{
    fn run(&self) {
        self.running.store(
            RunnableProcessorState::Running as u8,
            std::sync::atomic::Ordering::Release,
        );
        self.processor.handler.on_start();
        self.process_events();
        self.running.store(
            RunnableProcessorState::Idle as u8,
            std::sync::atomic::Ordering::Release,
        );
        self.processor.handler.on_shutdown();
    }

    fn stop(&mut self) {
        self.running.store(
            RunnableProcessorState::Halted as u8,
            std::sync::atomic::Ordering::Release,
        );
    }

    fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::Acquire)
            != RunnableProcessorState::Idle as u8
    }
}
