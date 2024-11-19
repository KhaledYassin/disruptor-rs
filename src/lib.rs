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

    const BUFFER_SIZE: usize = 1024;

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
            assert_eq!(*data, sequence);
        }

        fn on_start(&mut self) {}

        fn on_shutdown(&mut self) {}
    }

    #[test]
    fn test_dsl() {
        let data_provider = Arc::new(RingBuffer::new(BUFFER_SIZE));
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
        let data_provider = Arc::new(RingBuffer::new(BUFFER_SIZE));
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
        let num_elements = 1_000_000;
        let batch_size = 1000;
        let consumer_count = 3;
        let producer_count = 3;
        let data_provider = Arc::new(RingBuffer::new(BUFFER_SIZE));
        let (executor, producer) = builder::DisruptorBuilder::new(data_provider)
            .with_busy_spin_waiting_strategy()
            .with_multi_producer_sequencer()
            .with_barrier(|b| {
                for _ in 0..consumer_count {
                    b.handle_events_mut(Checker {});
                }
            })
            .build();

        let handle = executor.spawn();

        let producer_arc = Arc::new(producer);
        let mut producers: Vec<_> = vec![];
        for _ in 0..producer_count {
            let producer = Arc::clone(&producer_arc);
            let p = std::thread::spawn(move || {
                for chunk in (0..num_elements).step_by(batch_size) {
                    let end = (chunk + batch_size).min(num_elements);
                    let buffer = (chunk..end).collect::<Vec<_>>();
                    producer.write(buffer, |slot, seq, _| {
                        *slot = seq;
                    });
                }
            });
            producers.push(p);
        }

        for p in producers {
            p.join().unwrap();
        }
        if let Ok(producer) = Arc::try_unwrap(producer_arc) {
            producer.drain();
        }

        handle.join();
    }
}
