//! Event producers that write data into the ring buffer.
//!
//! # Producer Overview
//!
//! The Producer is responsible for writing events into the Disruptor's ring buffer in a
//! thread-safe manner. It coordinates with the Sequencer to obtain sequences and ensures
//! proper publication of events.
//!
//! # Key Components
//!
//! - **DataProvider**: Manages access to the underlying storage
//! - **Sequencer**: Coordinates sequence claims and publication
//! - **Event Writing**: Safe, efficient batch writing of events
//!
//! # Usage Example
//!
//! ``` rust
//! use my_disruptor::{RingBuffer, Producer, SingleProducerSequencer};
//!
//! // Create a producer
//! let producer = Producer::new(
//!     ring_buffer.clone(),
//!     sequencer
//! );
//!
//! // Write single event
//! producer.write(
//!     std::iter::once(42),
//!     |event, sequence, &value| {
//!         event.data = value;
//!     }
//! );
//!
//! // Write batch of events
//! let batch = vec![1, 2, 3, 4, 5];
//! producer.write(
//!     batch,
//!     |event, sequence, &value| {
//!         event.data = value;
//!     }
//! );
//! ```
//!
//! # Thread Safety
//!
//! The Producer is designed for single-producer scenarios. For multi-producer
//! scenarios, each producer should have its own instance, coordinated through
//! appropriate sequencer implementations.
//!
//! # Performance Considerations
//!
//! 1. **Batch Writing**
//!    - Prefer writing multiple events in a single call when possible
//!    - Reduces sequence claim overhead
//!    - Improves throughput
//!
//! 2. **Event Construction**
//!    - Keep event modification functions lightweight
//!    - Avoid blocking operations during event construction
//!
//! # Cleanup
//!
//! The `drain()` method ensures all events are properly published before
//! shutdown:
//!
//! ``` rust
//! // Ensure all events are published
//! producer.drain();
//! ```

use std::sync::Arc;

use crate::{
    sequence::Sequence,
    traits::{DataProvider, EventProducer, Sequencer},
};

pub struct Producer<D: DataProvider<T>, T, S: Sequencer> {
    data_provider: Arc<D>,
    sequencer: S,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, D: DataProvider<T> + 'a, T, S: Sequencer + 'a> EventProducer<'a> for Producer<D, T, S> {
    type Item = T;

    fn write<F, U, I, E>(&mut self, items: I, f: F)
    where
        I: IntoIterator<Item = U, IntoIter = E>,
        E: ExactSizeIterator<Item = U>,
        F: Fn(&mut Self::Item, Sequence, &U),
    {
        let iter = items.into_iter();
        let (start, end) = self.sequencer.next(iter.len() as Sequence);
        for (i, item) in iter.enumerate() {
            let sequence = start + i as Sequence;
            // SAFETY: The sequence is guaranteed to be within the bounds of the ring buffer.
            let data = unsafe { self.data_provider.get_mut(sequence) };
            f(data, sequence, &item);
        }
        self.sequencer.publish(start, end);
    }

    fn drain(self) {
        self.sequencer.drain();
    }
}

impl<D: DataProvider<T>, T, S: Sequencer> Producer<D, T, S> {
    pub fn new(data_provider: Arc<D>, sequencer: S) -> Self {
        Producer {
            data_provider,
            sequencer,
            _marker: Default::default(),
        }
    }
}
