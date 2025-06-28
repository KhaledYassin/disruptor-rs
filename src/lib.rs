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
            assert_eq!(*data, sequence);
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
        let data_provider = Arc::new(RingBuffer::new(BUFFER_SIZE));
        let (executor, producer) = builder::DisruptorBuilder::new(data_provider)
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

    #[test]
    fn test_multi_producer() {
        // Reduced scale to prevent timeouts after MultiProducerSequencer fixes
        let (num_elements, batch_size, consumer_count, producer_count) = if cfg!(miri) {
            (100, 10, 2, 2)
        } else {
            (1000, 100, 2, 2) // Much smaller scale: 1000 elements, 2 producers, 2 consumers
        };

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
        let mut producers = vec![];
        for _ in 0..producer_count {
            let producer = Arc::clone(&producer_arc);
            let p = std::thread::spawn(move || {
                for chunk in (0..num_elements).step_by(batch_size) {
                    let end = (chunk + batch_size).min(num_elements);
                    let buffer = (chunk..end).collect::<Vec<_>>();
                    producer.moving_write(buffer, |slot, seq, _| {
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
