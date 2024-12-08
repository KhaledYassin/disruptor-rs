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
//! # use disruptor_rs::{
//! #     processor::EventProcessorFactory,
//! #     traits::{EventHandler, Runnable},
//! #     sequence::Sequence,
//! # };
//!
//! // 1. Define your event handler
//! struct MyEvent;
//! struct MyHandler;
//!
//! impl EventHandler<MyEvent> for MyHandler {
//!     fn on_event(&self, _event: &MyEvent, sequence: Sequence, _end_of_batch: bool) {
//!         // Process the event
//!         println!("Processing event at sequence {}", sequence);
//!     }
//!
//!     // Optional lifecycle methods
//!     fn on_start(&self) {
//!         println!("Handler started");
//!     }
//!
//!     fn on_shutdown(&self) {}
//! }
//!
//! // 2. Create the processor
//! let handler = MyHandler;
//! let processor = EventProcessorFactory::create(handler);
//! ```
//!
//! ## Multiple Dependent Consumers
//! ```
//! use disruptor_rs::{
//!     processor::EventProcessorFactory,
//!     traits::{EventHandler, Runnable, DataProvider, EventProcessor, SequenceBarrier},
//!     sequence::Sequence,
//! };
//! use std::sync::Arc;
//!
//! // Define event type
//! struct MyEvent;
//!
//! // Define handlers
//! struct HandlerA;
//! impl EventHandler<MyEvent> for HandlerA {
//!     fn on_event(&self, _: &MyEvent, _: Sequence, _: bool) {}
//!     fn on_start(&self) {}
//!     fn on_shutdown(&self) {}
//! }
//!
//! struct HandlerB;
//! impl EventHandler<MyEvent> for HandlerB {
//!     fn on_event(&self, _: &MyEvent, _: Sequence, _: bool) {}
//!     fn on_start(&self) {}
//!     fn on_shutdown(&self) {}
//! }
//!
//! // Mock RingBuffer for example
//! struct MockRingBuffer;
//! impl DataProvider<MyEvent> for MockRingBuffer {
//!     fn get_capacity(&self) -> usize {
//!         1024 // Example fixed capacity
//!     }
//!
//!     unsafe fn get(&self, _: Sequence) -> &MyEvent {
//!         static EVENT: MyEvent = MyEvent;
//!         &EVENT
//!     }
//!     unsafe fn get_mut(&self, _: Sequence) -> &mut MyEvent {
//!         static mut EVENT: MyEvent = MyEvent;
//!         &mut EVENT
//!     }
//! }
//!
//! // Mock Barrier for example
//! struct MockBarrier;
//! impl SequenceBarrier for MockBarrier {
//!     fn wait_for(&self, seq: Sequence) -> Option<Sequence> {
//!         Some(seq)
//!     }
//!     fn signal(&self) {}
//! }
//!
//! // Create processors
//! let processor_a = EventProcessorFactory::create(HandlerA);
//! let processor_b = EventProcessorFactory::create(HandlerB);
//!
//! // Get processor A's sequence for processor B's barrier
//! let seq_a = processor_a.get_sequence();
//!
//! // Create mock ring buffer
//! let ring_buffer = Arc::new(MockRingBuffer);
//!
//! // Create barriers
//! let barrier_a = MockBarrier;
//! let barrier_b = MockBarrier;
//!
//! // Create runnables
//! let runnable_a = processor_a.create(ring_buffer.clone(), barrier_a);
//! let runnable_b = processor_b.create(ring_buffer.clone(), barrier_b);
//!
//! // Example of running processors (commented out to avoid actual thread creation in doc tests)
//! // std::thread::spawn(move || runnable_a.run());
//! // std::thread::spawn(move || runnable_b.run());
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
//! # use disruptor_rs::{
//! #     traits::EventHandler,
//! #     sequence::Sequence,
//! # };
//!
//! struct MyEvent;
//! struct MyHandler;
//!
//! impl MyHandler {
//!     fn process_event(&self, event: &MyEvent) -> Result<(), Box<dyn std::error::Error>> {
//!         Ok(())
//!     }
//! }
//!
//! impl EventHandler<MyEvent> for MyHandler {
//!     fn on_event(&self, event: &MyEvent, sequence: Sequence, _end_of_batch: bool) {
//!         match self.process_event(event) {
//!             Ok(_) => {
//!                 // Normal processing
//!             }
//!             Err(e) => {
//!                 // Log error but continue processing
//!                 println!("Error processing event at {}: {:?}", sequence, e);
//!             }
//!         }
//!     }
//!
//!     fn on_start(&self) {}
//!     fn on_shutdown(&self) {}
//! }
//! ```
//!
//! # Shutdown Handling
//!
//! Proper shutdown sequence:
//! ```
//! use disruptor_rs::{
//!     processor::EventProcessorFactory,
//!     traits::{EventHandler, Runnable, DataProvider, EventProcessor, SequenceBarrier},
//!     sequence::Sequence,
//! };
//! use std::sync::Arc;
//!
//! struct MyEvent;
//! struct MyHandler;
//!
//! impl EventHandler<MyEvent> for MyHandler {
//!     fn on_event(&self, _: &MyEvent, _: Sequence, _: bool) {}
//!     fn on_start(&self) {}
//!     fn on_shutdown(&self) {}
//! }
//!
//! struct MockBarrier;
//! impl SequenceBarrier for MockBarrier {
//!     fn wait_for(&self, seq: Sequence) -> Option<Sequence> {
//!         Some(seq)
//!     }
//!     fn signal(&self) {}
//! }
//!
//! // Mock RingBuffer for example
//! struct MockRingBuffer;
//! impl DataProvider<MyEvent> for MockRingBuffer {
//!     fn get_capacity(&self) -> usize {
//!         1024 // Example fixed capacity
//!     }
//!     unsafe fn get(&self, _: Sequence) -> &MyEvent {
//!         static EVENT: MyEvent = MyEvent;
//!         &EVENT
//!     }
//!     unsafe fn get_mut(&self, _: Sequence) -> &mut MyEvent {
//!         static mut EVENT: MyEvent = MyEvent;
//!         &mut EVENT
//!     }
//! }
//!
//! let barrier = MockBarrier;
//! let ring_buffer = Arc::new(MockRingBuffer);
//! let processor = EventProcessorFactory::create(MyHandler);
//! let mut runnable = processor.create(ring_buffer, barrier);
//!
//! // Signal shutdown
//! runnable.stop();
//!
//! // Wait for processing to complete
//! while runnable.is_running() {
//!     std::thread::sleep(std::time::Duration::from_millis(1000));
//! }
//! ```
//!
//! # Mutable Event Handlers
//!
//! For handlers that need to maintain mutable state:
//! ```rust
//! use disruptor_rs::{
//!     processor::EventProcessorFactory,
//!     traits::{EventHandlerMut, Runnable},
//!     sequence::Sequence,
//! };
//!
//! struct MyEvent;
//!
//! struct StatefulHandler {
//!     processed_count: usize,
//! }
//!
//! impl EventHandlerMut<MyEvent> for StatefulHandler {
//!     fn on_event(&mut self, _event: &MyEvent, sequence: Sequence, _end_of_batch: bool) {
//!         self.processed_count += 1;
//!         println!("Processed {} events, current sequence: {}", self.processed_count, sequence);
//!     }
//!
//!     fn on_start(&mut self) {
//!         self.processed_count = 0;
//!         println!("Starting stateful handler");
//!     }
//!
//!     fn on_shutdown(&mut self) {
//!         println!("Shutting down after processing {} events", self.processed_count);
//!     }
//! }
//!
//! // Create a mutable processor
//! let handler = StatefulHandler { processed_count: 0 };
//! let processor = EventProcessorFactory::create_mut(handler);
//! ```

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use crate::{
    sequence::AtomicSequence,
    traits::{DataProvider, EventHandler, EventProcessor, Runnable, SequenceBarrier},
    EventHandlerMut, EventProcessorMut,
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

    pub fn create_mut<'a, E, T>(handler: E) -> impl EventProcessorMut<'a, T>
    where
        E: EventHandlerMut<T> + Send + 'a,
        T: Send + 'a,
    {
        ProcessorMut {
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

struct ProcessorMut<E, T> {
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

struct RunnableProcessorMut<E, T, D: DataProvider<T>, B: SequenceBarrier> {
    running: AtomicU8,
    processor: ProcessorMut<E, T>,
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

impl<E, T, D, B> RunnableProcessorMut<E, T, D, B>
where
    E: EventHandlerMut<T> + Send,
    D: DataProvider<T>,
    B: SequenceBarrier,
    T: Send,
{
    fn process_events(&mut self) {
        let f = &mut self.processor.handler;
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

impl<'a, E, T> EventProcessorMut<'a, T> for ProcessorMut<E, T>
where
    E: EventHandlerMut<T> + Send + 'a,
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
        Box::new(RunnableProcessorMut {
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
    fn run(&mut self) {
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
            == RunnableProcessorState::Running as u8
    }
}

impl<E, T, D, B> Runnable for RunnableProcessorMut<E, T, D, B>
where
    E: EventHandlerMut<T> + Send,
    D: DataProvider<T>,
    B: SequenceBarrier,
    T: Send,
{
    fn run(&mut self) {
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
            == RunnableProcessorState::Running as u8
    }
}
