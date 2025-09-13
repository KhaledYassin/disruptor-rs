//! Worker-pool processors providing exactly-once consumption semantics.
//!
//! A worker pool splits the stream across N workers, ensuring each sequence
//! is processed by exactly one worker. Workers coordinate using a shared
//! `work_sequence` counter to claim the next sequence to process. Each worker
//! waits for publication using a barrier against the sequencer's cursor or
//! prior-stage consumers.

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use crate::{
    sequence::AtomicSequence,
    traits::{
        DataProvider, EventHandler, EventHandlerMut, EventProcessor, EventProcessorMut, Runnable,
        SequenceBarrier,
    },
};

/// Factory for creating work processors (exactly-once) from handlers.
pub struct WorkProcessorFactory;

impl WorkProcessorFactory {
    pub fn create<'a, E, T>(
        handler: E,
        work_sequence: Arc<AtomicSequence>,
    ) -> WorkEventProcessor<E, T>
    where
        E: EventHandler<T> + Send + 'a,
        T: Send + 'a,
    {
        WorkEventProcessor {
            handler,
            cursor: Default::default(),
            work_sequence,
            _marker: Default::default(),
        }
    }

    pub fn create_mut<'a, E, T>(
        handler: E,
        work_sequence: Arc<AtomicSequence>,
    ) -> WorkEventProcessorMut<E, T>
    where
        E: EventHandlerMut<T> + Send + 'a,
        T: Send + 'a,
    {
        WorkEventProcessorMut {
            handler,
            cursor: Default::default(),
            work_sequence,
            _marker: Default::default(),
        }
    }
}

/// EventProcessor that claims work from a shared `work_sequence`.
pub struct WorkEventProcessor<E, T> {
    handler: E,
    cursor: Arc<AtomicSequence>,
    work_sequence: Arc<AtomicSequence>,
    _marker: std::marker::PhantomData<T>,
}

/// EventProcessorMut that claims work from a shared `work_sequence`.
pub struct WorkEventProcessorMut<E, T> {
    handler: E,
    cursor: Arc<AtomicSequence>,
    work_sequence: Arc<AtomicSequence>,
    _marker: std::marker::PhantomData<T>,
}

struct WorkRunnable<E, T, D: DataProvider<T>, B: SequenceBarrier> {
    running: AtomicU8,
    processor: WorkEventProcessor<E, T>,
    data_provider: Arc<D>,
    barrier: B,
}

struct WorkRunnableMut<E, T, D: DataProvider<T>, B: SequenceBarrier> {
    running: AtomicU8,
    processor: WorkEventProcessorMut<E, T>,
    data_provider: Arc<D>,
    barrier: B,
}

impl<'a, E, T> EventProcessor<'a, T> for WorkEventProcessor<E, T>
where
    E: EventHandler<T> + Send + 'a,
    T: Send + 'a,
{
    fn get_cursor(&self) -> Arc<AtomicSequence> {
        self.cursor.clone()
    }

    fn create<D: DataProvider<T> + 'a, B: SequenceBarrier + 'a>(
        self,
        data_provider: Arc<D>,
        barrier: B,
    ) -> Box<dyn Runnable + 'a> {
        Box::new(WorkRunnable {
            running: AtomicU8::new(0),
            processor: self,
            data_provider,
            barrier,
        })
    }

    fn get_sequence(&self) -> Arc<AtomicSequence> {
        self.cursor.clone()
    }
}

impl<'a, E, T> EventProcessorMut<'a, T> for WorkEventProcessorMut<E, T>
where
    E: EventHandlerMut<T> + Send + 'a,
    T: Send + 'a,
{
    fn get_cursor(&self) -> Arc<AtomicSequence> {
        self.cursor.clone()
    }

    fn create<D: DataProvider<T> + 'a, B: SequenceBarrier + 'a>(
        self,
        data_provider: Arc<D>,
        barrier: B,
    ) -> Box<dyn Runnable + 'a> {
        Box::new(WorkRunnableMut {
            running: AtomicU8::new(0),
            processor: self,
            data_provider,
            barrier,
        })
    }

    fn get_sequence(&self) -> Arc<AtomicSequence> {
        self.cursor.clone()
    }
}

impl<E, T, D, B> WorkRunnable<E, T, D, B>
where
    E: EventHandler<T> + Send,
    D: DataProvider<T>,
    B: SequenceBarrier,
    T: Send,
{
    fn process_loop(&self) {
        let handler = &self.processor.handler;
        let cursor = &self.processor.cursor;
        let data_provider = &self.data_provider;
        let barrier = &self.barrier;
        let work_sequence = &self.processor.work_sequence;

        // Running state: 2 == Running (mirrors processor.rs)
        while self.running.load(Ordering::Acquire) == 2 {
            // Claim next sequence to process
            let claimed = work_sequence.increment_and_get();

            // Wait for data to be available up to at least `claimed`
            match barrier.wait_for(claimed) {
                Some(available) if available >= claimed => {
                    // SAFETY: Sequencer guarantees bounds; claim uniqueness is ensured by the atomic work counter
                    let event = unsafe { data_provider.get(claimed) };
                    handler.on_event(event, claimed, true);
                    cursor.set(claimed);
                    barrier.signal();
                }
                Some(_) => {
                    // Rare; loop will retry quickly
                }
                None => return, // Shutdown
            }
        }
    }
}

impl<E, T, D, B> Runnable for WorkRunnable<E, T, D, B>
where
    E: EventHandler<T> + Send,
    D: DataProvider<T>,
    B: SequenceBarrier,
    T: Send,
{
    fn run(&mut self) {
        self.running.store(2, Ordering::Release);
        self.processor.handler.on_start();
        self.process_loop();
        self.running.store(0, Ordering::Release);
        self.processor.handler.on_shutdown();
    }

    fn stop(&mut self) {
        self.running.store(1, Ordering::Release);
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire) == 2
    }
}

impl<E, T, D, B> WorkRunnableMut<E, T, D, B>
where
    E: EventHandlerMut<T> + Send,
    D: DataProvider<T>,
    B: SequenceBarrier,
    T: Send,
{
    fn process_loop(&mut self) {
        let handler = &mut self.processor.handler;
        let cursor = &self.processor.cursor;
        let data_provider = &self.data_provider;
        let barrier = &self.barrier;
        let work_sequence = &self.processor.work_sequence;

        while self.running.load(Ordering::Acquire) == 2 {
            let claimed = work_sequence.increment_and_get();

            match barrier.wait_for(claimed) {
                Some(available) if available >= claimed => {
                    let event = unsafe { data_provider.get(claimed) };
                    handler.on_event(event, claimed, true);
                    cursor.set(claimed);
                    barrier.signal();
                }
                Some(_) => {}
                None => return,
            }
        }
    }
}

impl<E, T, D, B> Runnable for WorkRunnableMut<E, T, D, B>
where
    E: EventHandlerMut<T> + Send,
    D: DataProvider<T>,
    B: SequenceBarrier,
    T: Send,
{
    fn run(&mut self) {
        self.running.store(2, Ordering::Release);
        self.processor.handler.on_start();
        self.process_loop();
        self.running.store(0, Ordering::Release);
        self.processor.handler.on_shutdown();
    }

    fn stop(&mut self) {
        self.running.store(1, Ordering::Release);
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire) == 2
    }
}
