# Disruptor-rs

[![Build Status](https://github.com/khaledyassin/disruptor-rs/workflows/Rust/badge.svg)](https://github.com/khaledyassin/disruptor-rs/actions/workflows/ci.yml)
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
