mod barrier;
mod builder;
mod executor;
pub mod processor;
pub mod producer;
pub mod ringbuffer;
pub mod sequence;
pub mod sequencer;
pub mod traits;
pub mod utils;
pub mod waiting;

pub use barrier::*;
pub use builder::*;
pub use traits::*;

#[cfg(test)]
mod tests {
    use ringbuffer::RingBuffer;
    use sequence::Sequence;
    use std::sync::Arc;
    use traits::{EventHandler, EventProcessorExecutor, EventProducer, ExecutorHandle};

    use super::*;

    struct Checker;
    impl EventHandler<i64> for Checker {
        fn on_event(&self, data: &i64, sequence: Sequence, _: bool) {
            if *data != sequence {
                dbg!(*data);
                dbg!(sequence);
                panic!();
            }
        }

        fn on_start(&self) {}

        fn on_shutdown(&self) {}
    }

    impl EventHandlerMut<i64> for Checker {
        fn on_event(&mut self, data: &i64, sequence: Sequence, _: bool) {
            if *data != sequence {
                dbg!(*data);
                dbg!(sequence);
                panic!();
            }
        }

        fn on_start(&mut self) {}

        fn on_shutdown(&mut self) {}
    }

    #[test]
    fn test_dsl() {
        let data_provider = Arc::new(RingBuffer::new(4096));
        let (executor, mut producer) = builder::DisruptorBuilder::new(data_provider)
            .with_busy_spin_waiting_strategy()
            .with_single_producer_sequencer()
            .with_barrier(|b| {
                b.handle_events(Checker {});
            })
            .build();

        let handle = executor.spawn();
        // Miri is much slower; use smaller batches under `cfg(miri)`
        let outer = if cfg!(miri) { 100 } else { 10_000 };
        let inner = if cfg!(miri) { 10 } else { 1_000 };

        for _ in 0..outer {
            let buffer: Vec<_> = std::iter::repeat_n(1, inner).collect();
            producer.write(buffer, |slot, seq, _| {
                *slot = seq;
            });
        }
        println!("Draining");
        producer.drain();
        handle.join();
    }

    #[test]
    fn test_dsl_mut() {
        let data_provider = Arc::new(RingBuffer::new(4096));
        let (executor, mut producer) = builder::DisruptorBuilder::new(data_provider)
            .with_busy_spin_waiting_strategy()
            .with_single_producer_sequencer()
            .with_barrier(|b| {
                b.handle_events_mut(Checker {});
            })
            .build();

        let handle = executor.spawn();
        // Miri is much slower; use smaller batches under `cfg(miri)`
        let outer = if cfg!(miri) { 100 } else { 10_000 };
        let inner = if cfg!(miri) { 10 } else { 1_000 };

        for _ in 0..outer {
            let buffer: Vec<_> = std::iter::repeat_n(1, inner).collect();
            producer.write(buffer, |slot, seq, _| {
                *slot = seq;
            });
        }
        println!("Draining");
        producer.drain();
        handle.join();
    }
}
