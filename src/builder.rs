use std::sync::Arc;

use crate::{
    executor::ThreadedExecutor,
    processor::EventProcessorFactory,
    producer::Producer,
    ringbuffer::RingBuffer,
    sequence::AtomicSequence,
    sequencer::{MultiProducerSequencer, SingleProducerSequencer},
    traits::{
        DataProvider, EventHandler, EventProcessor, EventProcessorExecutor, EventProducer,
        Runnable, Sequencer, WaitingStrategy,
    },
    waiting::{BusySpinWaitStrategy, SleepingWaitStrategy, YieldingWaitStrategy},
    EventHandlerMut, EventProcessorMut,
};

/// # Disruptor Builder Pattern Guide
///
/// The builder follows a type-state pattern to ensure compile-time correctness.
/// Each step in the builder chain enforces required configuration in a specific order:
///
/// 1. Start with data provider (ring buffer)
/// 2. Configure waiting strategy
/// 3. Set up sequencer
/// 4. Add event handlers
/// 5. Build the final disruptor
///
/// ## Example Usage
/// ```rust
/// use disruptor_rs::{
///     DisruptorBuilder, EventHandler, EventProcessorExecutor, EventProducer,
///     sequence::Sequence,
/// };
///
/// #[derive(Default)]
/// struct MyEvent;
///
/// #[derive(Default)]
/// struct MyEventHandler;
///
/// impl EventHandler<MyEvent> for MyEventHandler {
///     fn on_event(&self, _event: &MyEvent, _sequence: Sequence, _end_of_batch: bool) {}
///     fn on_start(&self) {}
///     fn on_shutdown(&self) {}
/// }
///
/// let (executor, producer) = DisruptorBuilder::with_ring_buffer::<MyEvent>(1024)
///     .with_busy_spin_waiting_strategy()
///     .with_single_producer_sequencer()
///     .with_barrier(|scope| {
///         scope.handle_events(MyEventHandler::default());
///     })
///     .build();
/// ```
///
/// ## Builder States
/// - `WithDataProvider`: Initial state, holds the ring buffer
/// - `WithWaitingStrategy`: Configures how consumers wait for new events
/// - `WithSequencer`: Manages the sequencing of events
/// - `WithEventHandlers`: Configures event processing chain
///
/// ## Barrier Scopes
/// The `with_barrier` method creates scopes for configuring event handlers
/// in a dependency chain. Handlers within the same barrier scope can run
/// in parallel, while different barrier scopes run sequentially.
///
/// ```rust
/// use disruptor_rs::{
///     DisruptorBuilder, EventHandler, EventProcessorExecutor, EventProducer,
///     sequence::Sequence,
/// };
///
/// #[derive(Default)]
/// struct MyEvent;
///
/// #[derive(Default)]
/// struct MyEventHandler;
///
/// impl EventHandler<MyEvent> for MyEventHandler {
///     fn on_event(&self, _event: &MyEvent, _sequence: Sequence, _end_of_batch: bool) {}
///     fn on_start(&self) {}
///     fn on_shutdown(&self) {}
/// }
///
/// let (executor, producer) = DisruptorBuilder::with_ring_buffer::<MyEvent>(1024)
///     .with_busy_spin_waiting_strategy()
///     .with_single_producer_sequencer()
///     .with_barrier(|scope| {
///         // These handlers run in parallel
///         scope.handle_events(MyEventHandler::default());
///         scope.handle_events(MyEventHandler::default());
///     })
///     .with_barrier(|scope| {
///         // This handler waits for both previous handlers
///         scope.handle_events(MyEventHandler::default());
///     })
///     .build();
/// ```

#[derive(Debug)]
pub struct DisruptorBuilder {}

pub struct WithDataProvider<D: DataProvider<T>, T>
where
    T: Send + Sync,
{
    data_provider: Arc<D>,
    _marker: std::marker::PhantomData<T>,
}

pub struct WithWaitingStrategy<W: WaitingStrategy, D: DataProvider<T>, T>
where
    T: Send + Sync,
{
    with_data_provider: WithDataProvider<D, T>,
    _waiting_strategy: std::marker::PhantomData<W>,
}

pub struct WithSequencer<S: Sequencer, W: WaitingStrategy, D: DataProvider<T>, T>
where
    T: Send + Sync,
{
    with_waiting_strategy: WithWaitingStrategy<W, D, T>,
    sequencer: S,
}

pub struct BarrierScope<'a, S: Sequencer, D: DataProvider<T>, T> {
    sequencer: S,
    data_provider: Arc<D>,
    gating_sequences: Vec<Arc<AtomicSequence>>,
    cursors: Vec<Arc<AtomicSequence>>,
    event_handlers: Vec<Box<dyn Runnable + 'a>>,
    _element: std::marker::PhantomData<T>,
}

pub struct WithEventHandlers<'a, S: Sequencer, W: WaitingStrategy, D: DataProvider<T>, T>
where
    T: Send + Sync,
{
    with_sequencer: WithSequencer<S, W, D, T>,
    event_handlers: Vec<Box<dyn Runnable + 'a>>,
    gating_sequences: Vec<Arc<AtomicSequence>>,
}

impl DisruptorBuilder {
    #[allow(clippy::new_ret_no_self)]
    pub fn new<D: DataProvider<T>, T>(data_provider: Arc<D>) -> WithDataProvider<D, T>
    where
        T: Send + Sync,
    {
        WithDataProvider {
            data_provider,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn with_ring_buffer<T>(capacity: usize) -> WithDataProvider<RingBuffer<T>, T>
    where
        T: Default + Send + Sync,
    {
        Self::new(Arc::new(RingBuffer::new(capacity)))
    }
}

impl<D: DataProvider<T>, T> WithDataProvider<D, T>
where
    T: Send + Sync,
{
    pub fn with_waiting_strategy<W: WaitingStrategy>(self) -> WithWaitingStrategy<W, D, T> {
        WithWaitingStrategy {
            with_data_provider: self,
            _waiting_strategy: Default::default(),
        }
    }

    pub fn with_busy_spin_waiting_strategy(
        self,
    ) -> WithWaitingStrategy<BusySpinWaitStrategy, D, T> {
        self.with_waiting_strategy()
    }

    pub fn with_yielding_waiting_strategy(self) -> WithWaitingStrategy<YieldingWaitStrategy, D, T> {
        self.with_waiting_strategy()
    }

    pub fn with_sleeping_waiting_strategy(self) -> WithWaitingStrategy<SleepingWaitStrategy, D, T> {
        self.with_waiting_strategy()
    }
}

impl<W: WaitingStrategy, D: DataProvider<T>, T> WithWaitingStrategy<W, D, T>
where
    T: Send + Sync,
{
    pub fn with_sequencer<S: Sequencer>(self, sequencer: S) -> WithSequencer<S, W, D, T> {
        WithSequencer {
            with_waiting_strategy: self,
            sequencer,
        }
    }

    pub fn with_single_producer_sequencer(
        self,
    ) -> WithSequencer<SingleProducerSequencer<W>, W, D, T> {
        let buffer_size = self.with_data_provider.data_provider.get_capacity();
        self.with_sequencer(SingleProducerSequencer::new(buffer_size, W::new()))
    }

    pub fn with_multi_producer_sequencer(
        self,
    ) -> WithSequencer<MultiProducerSequencer<W>, W, D, T> {
        let buffer_size = self.with_data_provider.data_provider.get_capacity();
        self.with_sequencer(MultiProducerSequencer::new(buffer_size, W::new()))
    }
}

impl<'a, S: Sequencer + 'a, W: WaitingStrategy, D: DataProvider<T> + 'a, T: Send + Sync + 'a>
    WithSequencer<S, W, D, T>
where
    T: Send + Sync,
{
    pub fn with_barrier(
        mut self,
        f: impl FnOnce(&mut BarrierScope<'a, S, D, T>),
    ) -> WithEventHandlers<'a, S, W, D, T> {
        let cursor = self.sequencer.get_cursor();
        let mut scope = BarrierScope {
            sequencer: self.sequencer,
            data_provider: self
                .with_waiting_strategy
                .with_data_provider
                .data_provider
                .clone(),
            gating_sequences: vec![cursor],
            event_handlers: Vec::new(),
            cursors: Vec::new(),
            _element: Default::default(),
        };

        f(&mut scope);
        self.sequencer = scope.sequencer;

        WithEventHandlers {
            with_sequencer: self,
            event_handlers: scope.event_handlers,
            gating_sequences: scope.cursors,
        }
    }
}

impl<'a, S: Sequencer + 'a, D: DataProvider<T> + 'a, T: Send + 'a> BarrierScope<'a, S, D, T> {
    pub fn handle_events<E>(&mut self, handler: E)
    where
        E: EventHandler<T> + Send + 'a,
    {
        self.handle_events_with(EventProcessorFactory::create(handler));
    }

    pub fn handle_events_mut<E>(&mut self, handler: E)
    where
        E: EventHandlerMut<T> + Send + 'a,
    {
        self.handle_events_with_mut(EventProcessorFactory::create_mut(handler));
    }

    pub fn handle_events_with<E: EventProcessor<'a, T>>(&mut self, processor: E) {
        self.cursors.push(processor.get_cursor());
        let barrier = self
            .sequencer
            .create_sequence_barrier(&self.gating_sequences);

        let runnable = processor.create(self.data_provider.clone(), barrier);
        self.event_handlers.push(runnable);
    }

    pub fn handle_events_with_mut<E: EventProcessorMut<'a, T>>(&mut self, processor: E) {
        self.cursors.push(processor.get_cursor());
        let barrier = self
            .sequencer
            .create_sequence_barrier(&self.gating_sequences);

        let runnable = processor.create(self.data_provider.clone(), barrier);
        self.event_handlers.push(runnable);
    }

    // Worker-pool APIs removed

    pub fn with_barrier(mut self, f: impl FnOnce(&mut BarrierScope<'a, S, D, T>)) {
        let mut scope = BarrierScope {
            sequencer: self.sequencer,
            data_provider: self.data_provider.clone(),
            gating_sequences: self.cursors,
            event_handlers: Vec::new(),
            cursors: Vec::new(),
            _element: Default::default(),
        };

        f(&mut scope);
        self.event_handlers.append(&mut scope.event_handlers);
    }
}

impl<'a, S: Sequencer + 'a, W: WaitingStrategy, D: DataProvider<T> + 'a, T: Send + Sync + 'a>
    WithEventHandlers<'a, S, W, D, T>
where
    T: Send + Sync,
{
    pub fn with_barrier(
        mut self,
        f: impl FnOnce(&mut BarrierScope<'a, S, D, T>),
    ) -> WithEventHandlers<'a, S, W, D, T> {
        let mut scope = BarrierScope {
            gating_sequences: self.gating_sequences.clone(),
            cursors: Vec::new(),
            sequencer: self.with_sequencer.sequencer,
            data_provider: self
                .with_sequencer
                .with_waiting_strategy
                .with_data_provider
                .data_provider
                .clone(),
            event_handlers: Vec::new(),
            _element: Default::default(),
        };

        f(&mut scope);
        self.with_sequencer.sequencer = scope.sequencer;
        self.event_handlers.append(&mut scope.event_handlers);
        self.gating_sequences = scope.cursors;

        self
    }

    pub fn build(
        self,
    ) -> (
        impl EventProcessorExecutor<'a>,
        impl EventProducer<'a, Item = T>,
    ) {
        self.build_with_executor::<ThreadedExecutor<'a>>()
    }

    pub fn build_with_executor<E: EventProcessorExecutor<'a>>(
        mut self,
    ) -> (E, impl EventProducer<'a, Item = T>) {
        for gs in &self.gating_sequences {
            self.with_sequencer.sequencer.add_gating_sequence(gs);
        }
        let executor = E::with_runnables(self.event_handlers);
        let producer = Producer::new(
            self.with_sequencer
                .with_waiting_strategy
                .with_data_provider
                .data_provider
                .clone(),
            self.with_sequencer.sequencer,
        );
        (executor, producer)
    }
}
