mod barrier;
mod builder;
mod executor;
mod processor;
mod producer;
mod ringbuffer;
mod sequence;
mod sequencer;
mod traits;
mod utils;
mod waiting;

pub use builder::*;
pub use traits::*;
pub mod internal {
    pub use super::barrier::*;
    pub use super::executor::*;
    pub use super::processor::*;
    pub use super::producer::*;
    pub use super::ringbuffer::*;
    pub use super::waiting::*;
}

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use ringbuffer::RingBuffer;
    use sequence::Sequence;
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
        for _ in 0..10_000 {
            let buffer: Vec<_> = std::iter::repeat(1).take(1000).collect();
            producer.write(buffer, |slot, seq, _| {
                *slot = seq;
            });
        }
        println!("Draining");
        producer.drain();
        handle.join();
    }
}
