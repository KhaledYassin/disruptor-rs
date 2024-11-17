use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput,
};
use disruptor_rs::ringbuffer::RingBuffer;
use disruptor_rs::EventHandlerMut;
use disruptor_rs::{
    sequence::Sequence, DisruptorBuilder, EventHandler, EventProcessorExecutor, EventProducer,
    ExecutorHandle,
};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

const BUFFER_SIZE: usize = 65536;
const ELEMENTS: usize = 1_000_000;

const PRODUCER_COUNT: usize = 3;
const CONSUMER_COUNT: usize = 3;
struct Checker;

impl EventHandler<i64> for Checker {
    fn on_event(&self, _event: &i64, _sequence: Sequence, _end_of_batch: bool) {
        assert_eq!(*_event, _sequence);
    }

    fn on_start(&self) {}
    fn on_shutdown(&self) {}
}

impl EventHandlerMut<i64> for Checker {
    fn on_event(&mut self, _event: &i64, _sequence: Sequence, _end_of_batch: bool) {}

    fn on_start(&mut self) {}
    fn on_shutdown(&mut self) {}
}

fn throughput_single_producer_single_consumer(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc");
    group.throughput(Throughput::Elements(ELEMENTS as u64));
    group.warm_up_time(Duration::from_secs(10));
    group.sampling_mode(SamplingMode::Flat);

    for batch_size in [1, 10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &batch_size| {
                b.iter(|| {
                    let (tx, rx) = mpsc::channel();
                    let producer = thread::spawn(move || {
                        for chunk in (0..ELEMENTS).step_by(batch_size) {
                            let end = (chunk + batch_size).min(ELEMENTS);
                            let batch = (chunk..end).collect::<Vec<_>>();
                            tx.send(batch).unwrap();
                        }
                    });

                    let consumer = thread::spawn(move || {
                        while let Ok(batch) = rx.recv() {
                            black_box(batch);
                        }
                    });

                    let _ = producer.join();
                    let _ = consumer.join();
                });
            },
        );
    }
    group.finish();

    let mut group = c.benchmark_group("spsc_disruptor");
    group.throughput(Throughput::Elements(ELEMENTS as u64));
    group.warm_up_time(Duration::from_secs(10));
    group.sampling_mode(SamplingMode::Flat);
    for batch_size in [1, 10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &batch_size| {
                b.iter(|| {
                    let data_provider = Arc::new(RingBuffer::new(BUFFER_SIZE));
                    let (executor, producer) = DisruptorBuilder::new(data_provider)
                        .with_busy_spin_waiting_strategy()
                        .with_single_producer_sequencer()
                        .with_barrier(|b| {
                            b.handle_events(Checker {});
                        })
                        .build();

                    let handle = executor.spawn();
                    for chunk in (0..ELEMENTS).step_by(batch_size) {
                        let end = (chunk + batch_size).min(ELEMENTS);
                        let batch = (chunk..end).collect::<Vec<_>>();
                        producer.write(batch, |slot, seq, _| *slot = seq);
                    }
                    producer.drain();
                    handle.join();
                });
            },
        );
    }
    group.finish();
}

fn throughput_multi_producer_multi_consumer(c: &mut Criterion) {
    let mut group = c.benchmark_group("mpmc");
    group.throughput(Throughput::Elements(ELEMENTS as u64));
    group.warm_up_time(Duration::from_secs(10));
    group.sampling_mode(SamplingMode::Flat);

    for batch_size in [1, 10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &batch_size| {
                b.iter(|| {
                    let (tx, rx) = crossbeam_channel::bounded(BUFFER_SIZE);
                    let producers: Vec<_> = (0..PRODUCER_COUNT)
                        .map(|_| {
                            let sender = tx.clone();
                            thread::spawn(move || {
                                for chunk in (0..ELEMENTS / PRODUCER_COUNT).step_by(batch_size) {
                                    let end = (chunk + batch_size).min(ELEMENTS / PRODUCER_COUNT);
                                    let batch = (chunk..end).collect::<Vec<_>>();
                                    sender.send(batch).unwrap();
                                }
                            })
                        })
                        .collect();

                    let consumers: Vec<_> = (0..CONSUMER_COUNT)
                        .map(|_| {
                            let receiver = rx.clone();
                            thread::spawn(move || {
                                while let Ok(batch) = receiver.recv() {
                                    black_box(batch);
                                }
                            })
                        })
                        .collect();

                    // Wait for all producers to finish
                    for producer in producers {
                        let _ = producer.join();
                    }

                    // Close the channel after all producers are done
                    drop(tx); // This will close the sending end of the channel

                    // Wait for all consumers to finish
                    for consumer in consumers {
                        let _ = consumer.join();
                    }
                });
            },
        );
    }

    group.finish();

    let mut group = c.benchmark_group("mpmc_disruptor");
    group.throughput(Throughput::Elements(ELEMENTS as u64));
    group.warm_up_time(Duration::from_secs(10));
    group.sampling_mode(SamplingMode::Flat);

    for batch_size in [1, 10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &batch_size| {
                b.iter(|| {
                    let data_provider = Arc::new(RingBuffer::new(BUFFER_SIZE));
                    let (executor, producer) = DisruptorBuilder::new(data_provider)
                        .with_busy_spin_waiting_strategy()
                        .with_multi_producer_sequencer()
                        .with_barrier(|b| {
                            for _ in 0..CONSUMER_COUNT {
                                b.handle_events_mut(Checker {});
                            }
                        })
                        .build();

                    let handle = executor.spawn();
                    let producer = Arc::new(producer);

                    let mut producer_handles: Vec<_> = vec![];

                    for _ in 0..PRODUCER_COUNT {
                        let producer = producer.clone();
                        producer_handles.push(thread::spawn(move || {
                            for chunk in (0..ELEMENTS).step_by(batch_size) {
                                let end = (chunk + batch_size).min(ELEMENTS);
                                let batch = (chunk..end).collect::<Vec<_>>();
                                producer.write(batch, |slot, seq, _| *slot = seq);
                            }
                        }));
                    }

                    // Wait for all producers to finish
                    for producer_handle in producer_handles {
                        let _ = producer_handle.join();
                    }

                    // Drain the producer to ensure all messages are processed
                    if let Ok(producer) = Arc::try_unwrap(producer) {
                        producer.drain();
                    }

                    handle.join();
                });
            },
        );
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().measurement_time(Duration::from_secs(60)).sample_size(100);
    targets = throughput_single_producer_single_consumer, throughput_multi_producer_multi_consumer
}
criterion_main!(benches);
