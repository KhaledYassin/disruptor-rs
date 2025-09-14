# Disruptor-rs

[![Build Status](https://github.com/khaledyassin/disruptor-rs/workflows/CI/badge.svg)](https://github.com/khaledyassin/disruptor-rs/actions/workflows/ci.yml)
[![Code Coverage](https://codecov.io/gh/khaledyassin/disruptor-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/khaledyassin/disruptor-rs)
[![Crates.io](https://img.shields.io/crates/v/disruptor-rs.svg)](https://crates.io/crates/disruptor-rs)
[![Documentation](https://docs.rs/disruptor-rs/badge.svg)](https://docs.rs/disruptor-rs)
[![License](https://img.shields.io/crates/l/disruptor-rs.svg)](LICENSE)
[![Rust Version](https://img.shields.io/badge/rust-1.70%2B-blue.svg)](https://www.rust-lang.org)

A heavily documented Rust implementation of the LMAX Disruptor pattern focused on high-performance inter-thread messaging.

## Origins & Purpose

This project is a fork of [disrustor](https://github.com/sklose/disrustor) by Sebastian Klose. While the original implementation provided an excellent foundation, this fork aims to:

1. Provide comprehensive documentation explaining the Disruptor pattern
2. Rewrite the implementation in a more idiomatic Rust style
3. Create a production-ready, Rust-native package for the community

## Important Differences from the original implementation

The most important difference is that this implementation is that the `Sequencer` uses `WaitingStrategy` to get coordinate between the sequences instead of the default implied busy-spinning. This makes the disruptor more consistent and potentially more performant and efficient in its CPU usage.

## What is the Disruptor?

The Disruptor is a high-performance inter-thread messaging library, originally developed by LMAX Exchange for their financial trading platform. It achieves superior performance through:

- Lock-free algorithms
- Cache-line padding to prevent false sharing
- Efficient memory pre-allocation
- Mechanical sympathy with modern CPU architectures

## Key Components

### 1. Ring Buffer

A fixed-size circular buffer that pre-allocates memory for events:

- Zero garbage collection
- Cache-friendly memory access patterns
- Power-of-2 sizing for efficient modulo operations

### 2. Sequences

Cache-line padded atomic counters that track:

- Producer position in the ring buffer
- Consumer progress
- Dependencies between consumers

### 3. Sequencer

Coordinates access to the ring buffer:

- Manages sequence claim and publication
- Implements backpressure
- Ensures thread-safety

### 4. Event Processing

- **Producers**: Write events to the ring buffer
- **Consumers**: Process events through user-defined handlers
- **Barriers**: Coordinate dependencies between processors

### 5. Waiting Strategies

Different strategies for thread coordination:

- `BusySpinWaitStrategy`: Lowest latency, highest CPU usage
- `YieldingWaitStrategy`: Balanced approach
- `SleepingWaitStrategy`: Lowest CPU usage, higher latency

## Usage Example

```rust
use disruptor_rs::{DisruptorBuilder, EventHandler};
// Define your event type
#[derive(Default)]
struct MyEvent {
    data: i64,
}
// Define your event handler
#[derive(Default)]
struct MyHandler;
impl EventHandler<MyEvent> for MyHandler {
    fn on_event(&self, event: &MyEvent, sequence: i64, end_of_batch: bool) {
        println!("Processing event: {} at sequence {}", event.data, sequence);
    }
}
// Build and run the disruptor
let (executor, producer) = DisruptorBuilder::with_ring_buffer::<MyEvent>(1024)
    .with_busy_spin_waiting_strategy()
    .with_single_producer_sequencer()
    .with_barrier(|scope| {
        scope.handle_events(MyHandler::default());
    })
    .build();
// Start processing
let handle = executor.spawn();
// Produce events
// ...
// Cleanup
producer.drain();
handle.join();
```

## Performance Considerations

- **Ring Buffer Size**: Must be a power of 2
- **Waiting Strategy**: Choose based on your latency/CPU trade-offs
- **Event Handlers**: Keep processing logic lightweight
- **Batch Processing**: Use batch writes when possible

## A Note about the `MultiProducerSequencer`

The `MultiProducerSequencer` feature is not very stable yet. You may feel free to use it experimentally. It passes all the sequencing tests but it's not optimized enough to beat the impressive performance of the [crossbeam_channel](https://github.com/crossbeam-rs/crossbeam/tree/master/crossbeam-channel). Any contributions in this regard are more than welcome.

## Benchmarks

The repository includes Criterion benchmarks in `benches/throughput.rs` comparing:

- Crossbeam channels (SPSC and MPMC)
- Disruptor (SPSC and MPMC) with `BusySpinWaitStrategy`

### Methodology (from `benches/throughput.rs`)

- Buffer size: 1024 elements (`BUFFER_SIZE`)
- Elements per run: `ELEMENTS = BUFFER_SIZE`
- Producer/consumer counts for MPMC: 3 producers, 3 consumers
- Batch sizes tested: 1, 10, 100 elements
- Criterion configuration:
  - Warm-up: 5–10 seconds (depending on group)
  - Measurement time: 5–10 seconds (default group settings)
  - Sampling mode: Flat

Notes:

- Variance and outliers are reported by Criterion; see `target/criterion/*/report/index.html` for full visualizations.
- Numbers depend on hardware and OS. Use them for relative comparisons.

### How to run

```bash
# Run all tests first
cargo test

# Run throughput benchmarks
cargo bench --bench throughput
```

### Interpretation

- SPSC Disruptor shows strong efficiency at higher batch sizes due to contiguous scans and preallocated ring memory.
- MPMC Disruptor improves the send path by marking per-slot availability (no publish-side cursor advance). Receivers determine contiguity, similar to Crossbeam’s approach.

### Tabular comparison of the benchmark results

SPSC (std mpsc) vs Disruptor (median values):

| Batch | std mpsc Time | Disruptor Time | std mpsc Thrpt | Disruptor Thrpt |
| ----- | ------------- | -------------- | -------------- | --------------- |
| 1     | 176.61 µs     | 56.252 µs      | 5.7980 Melem/s | 18.204 Melem/s  |
| 10    | 32.749 µs     | 31.897 µs      | 31.268 Melem/s | 32.103 Melem/s  |
| 100   | 27.737 µs     | 20.300 µs      | 36.918 Melem/s | 50.444 Melem/s  |

MPMC (crossbeam) vs Disruptor (median values):

| Batch | Crossbeam Time | Disruptor Time | Crossbeam Thrpt | Disruptor Thrpt |
| ----- | -------------- | -------------- | --------------- | --------------- |
| 1     | 312.90 µs      | 173.65 µs      | 3.2726 Melem/s  | 5.8970 Melem/s  |
| 10    | 68.888 µs      | 106.74 µs      | 14.865 Melem/s  | 9.5937 Melem/s  |
| 100   | 64.921 µs      | 101.91 µs      | 15.773 Melem/s  | 10.048 Melem/s  |

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. Areas of interest:

- Additional waiting strategies
- Performance optimizations
- Documentation improvements
- Example use cases

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Acknowledgments

- Original implementation by [Sebastian Klose](https://github.com/sklose/disrustor)
- LMAX team for the [original Java implementation](https://github.com/LMAX-Exchange/disruptor)
