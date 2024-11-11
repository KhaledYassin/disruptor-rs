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
        let (executor, producer) = builder::DisruptorBuilder::new(data_provider)
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

    #[test]
    fn test_dsl_mut() {
        let data_provider = Arc::new(RingBuffer::new(4096));
        let (executor, producer) = builder::DisruptorBuilder::new(data_provider)
            .with_busy_spin_waiting_strategy()
            .with_single_producer_sequencer()
            .with_barrier(|b| {
                b.handle_events_mut(Checker {});
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

    #[test]
    fn test_multi_producer() {
        let data_provider = Arc::new(RingBuffer::new(4096));
        let (executor, producer) = builder::DisruptorBuilder::new(data_provider)
            .with_busy_spin_waiting_strategy()
            .with_multi_producer_sequencer()
            .with_barrier(|b| {
                b.handle_events_mut(Checker {});
            })
            .build();

        let handle = executor.spawn();

        let producer_arc = Arc::new(producer);
        let producer1 = producer_arc.clone();
        let producer2 = producer_arc.clone();
        let producer3 = producer_arc.clone();

        let p1 = std::thread::spawn(move || {
            for _ in 0..10_000 {
                let buffer: Vec<_> = std::iter::repeat(1).take(1000).collect();
                producer1.write(buffer, |slot, seq, _| {
                    *slot = seq;
                });
            }
        });

        let p2 = std::thread::spawn(move || {
            for _ in 0..10_000 {
                let buffer: Vec<_> = std::iter::repeat(2).take(1000).collect();
                producer2.write(buffer, |slot, seq, _| {
                    *slot = seq;
                });
            }
        });

        let p3 = std::thread::spawn(move || {
            for _ in 0..10_000 {
                let buffer: Vec<_> = std::iter::repeat(3).take(1000).collect();
                producer3.write(buffer, |slot, seq, _| {
                    *slot = seq;
                });
            }
        });

        p1.join().unwrap();
        p2.join().unwrap();
        p3.join().unwrap();
        if let Ok(producer) = Arc::try_unwrap(producer_arc) {
            producer.drain();
        }

        handle.join();
    }
}
