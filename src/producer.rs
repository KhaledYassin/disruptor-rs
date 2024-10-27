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
