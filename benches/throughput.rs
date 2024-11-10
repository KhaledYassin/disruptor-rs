use criterion::{black_box, criterion_group, criterion_main, Criterion};
use disruptor_rs::{
    sequence::Sequence, DisruptorBuilder, EventHandler, EventProcessorExecutor, EventProducer,
    ExecutorHandle,
};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

const BUFFER_SIZE: usize = 1024 * 16;
const ITERATIONS: usize = 1_000_000;
const BATCH_SIZE: usize = 100;
const PRODUCER_COUNT: usize = 3;
const CONSUMER_COUNT: usize = 3;
struct TestHandler {
    count: std::sync::atomic::AtomicUsize,
}

impl EventHandler<i64> for TestHandler {
    fn on_event(&self, _event: &i64, _sequence: Sequence, _end_of_batch: bool) {
        self.count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn on_start(&self) {}
    fn on_shutdown(&self) {}
}

fn bench_channel_spsc(c: &mut Criterion) {
    c.bench_function("channel_spsc", |b| {
        b.iter(|| {
            let (tx, rx) = mpsc::channel();
            let handle = thread::spawn(move || {
                for _ in 0..ITERATIONS {
                    black_box(rx.recv().unwrap());
                }
            });

            for i in 0..ITERATIONS {
                tx.send(black_box(i as i64)).unwrap();
            }
            handle.join().unwrap();
        })
    });
}

fn bench_disruptor_spsc(c: &mut Criterion) {
    c.bench_function("disruptor_spsc", |b| {
        b.iter(|| {
            let handler = TestHandler {
                count: std::sync::atomic::AtomicUsize::new(0),
            };

            let (executor, producer) = DisruptorBuilder::with_ring_buffer(BUFFER_SIZE)
                .with_busy_spin_waiting_strategy()
                .with_single_producer_sequencer()
                .with_barrier(|b| {
                    b.handle_events(handler);
                })
                .build();

            let handle = executor.spawn();

            // Batch write for better performance
            for chunk in (0..ITERATIONS).step_by(BATCH_SIZE) {
                let end = (chunk + BATCH_SIZE).min(ITERATIONS);
                let batch: Vec<_> = (chunk..end).map(|i| i as i64).collect();

                producer.write(batch, |slot, _seq, &value| {
                    *slot = value;
                });
            }

            producer.drain();
            handle.join();
        })
    });
}

fn bench_channel_spmc(c: &mut Criterion) {
    c.bench_function("channel_spmc", |b| {
        b.iter(|| {
            let (tx, rx) = crossbeam_channel::bounded(BUFFER_SIZE);
            let rx = std::sync::Arc::new(rx);

            let mut handles = vec![];
            for _ in 0..CONSUMER_COUNT {
                let rx = rx.clone();
                handles.push(thread::spawn(move || {
                    for _ in 0..ITERATIONS / CONSUMER_COUNT {
                        black_box(rx.recv().unwrap());
                    }
                }));
            }

            for i in 0..ITERATIONS {
                tx.send(black_box(i as i64)).unwrap();
            }

            for handle in handles {
                handle.join().unwrap();
            }
        })
    });
}

fn bench_disruptor_spmc(c: &mut Criterion) {
    c.bench_function("disruptor_spmc", |b| {
        b.iter(|| {
            let (executor, producer) = DisruptorBuilder::with_ring_buffer(BUFFER_SIZE)
                .with_busy_spin_waiting_strategy()
                .with_single_producer_sequencer()
                .with_barrier(|b| {
                    for _ in 0..CONSUMER_COUNT {
                        b.handle_events(TestHandler {
                            count: std::sync::atomic::AtomicUsize::new(0),
                        });
                    }
                })
                .build();

            let handle = executor.spawn();

            for chunk in (0..ITERATIONS).step_by(BATCH_SIZE) {
                let end = (chunk + BATCH_SIZE).min(ITERATIONS);
                let batch: Vec<_> = (chunk..end).map(|i| i as i64).collect();

                producer.write(batch, |slot, _seq, &value| {
                    *slot = value;
                });
            }

            producer.drain();
            handle.join();
        })
    });
}

fn bench_channel_mpmc(c: &mut Criterion) {
    c.bench_function("channel_mpmc", |b| {
        b.iter(|| {
            let (tx, rx) = crossbeam_channel::bounded(BUFFER_SIZE);
            let tx = std::sync::Arc::new(tx);
            let rx = std::sync::Arc::new(rx);

            let mut handles = vec![];
            // Spawn consumer threads
            for _ in 0..CONSUMER_COUNT {
                let rx: std::sync::Arc<crossbeam_channel::Receiver<i64>> = rx.clone();
                handles.push(thread::spawn(move || {
                    for _ in 0..ITERATIONS / CONSUMER_COUNT {
                        black_box(rx.recv().unwrap());
                    }
                }));
            }

            // Spawn producer threads
            let mut producer_handles = vec![];
            for _ in 0..PRODUCER_COUNT {
                let tx = tx.clone();
                producer_handles.push(thread::spawn(move || {
                    for i in 0..ITERATIONS / PRODUCER_COUNT {
                        tx.send(black_box(i as i64)).unwrap();
                    }
                }));
            }

            for handle in producer_handles {
                handle.join().unwrap();
            }

            for handle in handles {
                handle.join().unwrap();
            }
        })
    });
}

fn bench_disruptor_mpmc(c: &mut Criterion) {
    c.bench_function("disruptor_mpmc", |b| {
        b.iter(|| {
            let (executor, producer) = DisruptorBuilder::with_ring_buffer(BUFFER_SIZE)
                .with_busy_spin_waiting_strategy()
                .with_multi_producer_sequencer()
                .with_barrier(|b| {
                    for _ in 0..CONSUMER_COUNT {
                        b.handle_events(TestHandler {
                            count: std::sync::atomic::AtomicUsize::new(0),
                        });
                    }
                })
                .build();

            let handle = executor.spawn();
            let producer = std::sync::Arc::new(producer);

            // Spawn producer threads
            let mut producer_handles = vec![];
            for _ in 0..PRODUCER_COUNT {
                let producer = producer.clone();
                producer_handles.push(thread::spawn(move || {
                    for i in 0..ITERATIONS / PRODUCER_COUNT {
                        producer.write(vec![1], |slot, _, _| {
                            *slot = black_box(i as i64);
                        });
                    }
                }));
            }

            // Wait for producers to finish
            for handle in producer_handles {
                handle.join().unwrap();
            }

            // Drain and cleanup
            if let Ok(producer) = Arc::try_unwrap(producer) {
                producer.drain();
            }
            handle.join();
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(15))
        .sample_size(10);
    targets = bench_channel_spsc, bench_disruptor_spsc,
              bench_channel_spmc, bench_disruptor_spmc,
              bench_channel_mpmc, bench_disruptor_mpmc
}
criterion_main!(benches);
