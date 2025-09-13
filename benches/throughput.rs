use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput,
};
use disruptor_rs::ringbuffer::RingBuffer;
use disruptor_rs::EventHandlerMut;
use disruptor_rs::{
    sequence::Sequence, DisruptorBuilder, EventHandler, EventProcessorExecutor, EventProducer,
    ExecutorHandle,
};
use disruptor_rs::work::WorkProcessorFactory;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

const BUFFER_SIZE: usize = 1024;
const ELEMENTS: usize = BUFFER_SIZE;

const PRODUCER_COUNT: usize = 3;
const CONSUMER_COUNT: usize = 3;
struct Checker;

impl EventHandler<i64> for Checker {
    fn on_event(&self, event: &i64, _sequence: Sequence, _end_of_batch: bool) {
        black_box(*event);
    }

    fn on_start(&self) {}
    fn on_shutdown(&self) {}
}

impl EventHandlerMut<i64> for Checker {
    fn on_event(&mut self, event: &i64, _sequence: Sequence, _end_of_batch: bool) {
        black_box(*event);
    }
    fn on_start(&mut self) {}
    fn on_shutdown(&mut self) {}
}

fn throughput_single_producer_single_consumer(c: &mut Criterion) {
    let mut group = c.benchmark_group("spsc");
    group.throughput(Throughput::Elements(ELEMENTS as u64));
    group.warm_up_time(Duration::from_secs(5));
    group.measurement_time(Duration::from_secs(5));
    group.sampling_mode(SamplingMode::Flat);

    for batch_size in [1, 10, 100] {
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
    group.warm_up_time(Duration::from_secs(5));
    group.measurement_time(Duration::from_secs(5));
    group.sampling_mode(SamplingMode::Flat);
    for batch_size in [1, 10, 100] {
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

    for batch_size in [1, 10, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &batch_size| {
                b.iter(|| {
                    let (tx, rx) = crossbeam_channel::bounded(BUFFER_SIZE);
                    let elements_per_producer = ELEMENTS / PRODUCER_COUNT;
                    let producers: Vec<_> = (0..PRODUCER_COUNT)
                        .map(|producer_id| {
                            let sender = tx.clone();
                            let start_element = producer_id * elements_per_producer;
                            let end_element = if producer_id == PRODUCER_COUNT - 1 {
                                ELEMENTS // Last producer handles any remainder
                            } else {
                                (producer_id + 1) * elements_per_producer
                            };

                            thread::spawn(move || {
                                for chunk in (start_element..end_element).step_by(batch_size) {
                                    let end = (chunk + batch_size).min(end_element);
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
    group.warm_up_time(Duration::from_secs(5));
    group.measurement_time(Duration::from_secs(5));
    group.sampling_mode(SamplingMode::Flat);

    for batch_size in [1, 10, 100] {
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

                    let producer_arc = Arc::new(producer);
                    let mut producers = vec![];
                    let elements_per_producer = ELEMENTS / PRODUCER_COUNT;
                    for producer_id in 0..PRODUCER_COUNT {
                        let producer = Arc::clone(&producer_arc);
                        let start_element = producer_id * elements_per_producer;
                        let end_element = if producer_id == PRODUCER_COUNT - 1 {
                            ELEMENTS // Last producer handles any remainder
                        } else {
                            (producer_id + 1) * elements_per_producer
                        };

                        let p = std::thread::spawn(move || {
                            for chunk in (start_element..end_element).step_by(batch_size) {
                                let end = (chunk + batch_size).min(end_element);
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
                });
            },
        );
    }
    group.finish();

    // Worker-pool variant: exactly-once processing across N workers
    let mut group = c.benchmark_group("mpmc_disruptor_worker_pool");
    group.throughput(Throughput::Elements(ELEMENTS as u64));
    group.warm_up_time(Duration::from_secs(5));
    group.measurement_time(Duration::from_secs(5));
    group.sampling_mode(SamplingMode::Flat);

    for batch_size in [1, 10, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &batch_size| {
                b.iter(|| {
                    let data_provider = Arc::new(RingBuffer::new(BUFFER_SIZE));
                    let (executor, builder_producer) = DisruptorBuilder::new(data_provider)
                        .with_busy_spin_waiting_strategy()
                        .with_multi_producer_sequencer()
                        .with_barrier(|b| {
                            // Shared work_sequence across all workers in this pool
                            let work_sequence = Arc::new(disruptor_rs::sequence::AtomicSequence::new(-1));
                            for _ in 0..CONSUMER_COUNT {
                                b.handle_work_mut(Checker {}, &work_sequence);
                            }
                        })
                        .build();

                    let handle = executor.spawn();

                    let producer_arc = Arc::new(builder_producer);
                    let mut producers = vec![];
                    let elements_per_producer = ELEMENTS / PRODUCER_COUNT;
                    for producer_id in 0..PRODUCER_COUNT {
                        let producer = Arc::clone(&producer_arc);
                        let start_element = producer_id * elements_per_producer;
                        let end_element = if producer_id == PRODUCER_COUNT - 1 {
                            ELEMENTS
                        } else {
                            (producer_id + 1) * elements_per_producer
                        };

                        let p = std::thread::spawn(move || {
                            for chunk in (start_element..end_element).step_by(batch_size) {
                                let end = (chunk + batch_size).min(end_element);
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
                });
            },
        );
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().measurement_time(Duration::from_secs(10)).sample_size(10);
    targets =
            throughput_single_producer_single_consumer,
        throughput_multi_producer_multi_consumer
}
criterion_main!(benches);
