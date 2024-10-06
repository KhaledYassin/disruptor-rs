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
                        f.on_event(event, available_sequence, i == available_sequence);
                    }

                    cursor.set(available_sequence);
                    barrier.alert();
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
