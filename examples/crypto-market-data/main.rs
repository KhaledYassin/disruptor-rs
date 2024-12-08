//! # Crypto Market Data
//!
//! This example demonstrates how to use disruptor-rs to process incoming market data from a websocket feed.
//!
//! The example subscribes to a websocket feed for multiple symbols and processes the incoming messages in a
//! multi-producer, single-consumer configuration.
//!
//! To run the example, execute the following command:
//!
//! ```bash
//! cargo run --example crypto-market-data
//! ```

use binance::{BinanceMessage, WebsocketSubscriptionHandler};
use disruptor_rs::sequence::Sequence;
use disruptor_rs::{DisruptorBuilder, EventHandlerMut, EventProcessorExecutor, ExecutorHandle};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

mod binance;

struct MarketDataHandler;

impl EventHandlerMut<BinanceMessage> for MarketDataHandler {
    fn on_event(&mut self, _event: &BinanceMessage, _sequence: Sequence, _end_of_batch: bool) {
        println!("Received message: {:?}", _event);
    }

    fn on_start(&mut self) {
        println!("Starting MarketDataHandler");
    }

    fn on_shutdown(&mut self) {
        println!("Shutting down MarketDataHandler");
    }
}

fn main() {
    let (executor, producer) = DisruptorBuilder::with_ring_buffer(1024)
        .with_busy_spin_waiting_strategy()
        .with_single_producer_sequencer()
        .with_barrier(|b| {
            b.handle_events_mut(MarketDataHandler);
        })
        .build();
    let running = Arc::new(AtomicBool::new(true));
    let running_ctrlc = running.clone();

    let handle = executor.spawn();
    let producer_arc = Arc::new(producer);

    // Set up the Ctrl+C handler
    ctrlc::set_handler(move || {
        println!("Received Ctrl+C! Shutting down...");
        running_ctrlc.store(false, std::sync::atomic::Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let p = std::thread::spawn(move || {
        let mut binance = binance::BinanceWebsocketHandler::new(
            &[
                "btcusdt".to_string(),
                "ethusdt".to_string(),
                "solusdt".to_string(),
            ],
            running.clone(),
            producer_arc.clone(),
        );
        binance.start();
        binance.run();
        binance.stop();
    });

    p.join().unwrap();
    handle.join();
}
