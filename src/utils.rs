use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

use crate::sequence::AtomicSequence;

pub struct Utils;

impl Utils {
    pub fn get_minimum_sequence(sequences: &[Arc<AtomicSequence>]) -> i64 {
        if sequences.is_empty() {
            i64::MAX
        } else {
            sequences.iter().map(|s| s.get()).min().unwrap()
        }
    }

    pub fn get_maximum_sequence(sequences: &[Arc<AtomicSequence>]) -> i64 {
        if sequences.is_empty() {
            i64::MIN
        } else {
            sequences.iter().map(|s| s.get()).max().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::AtomicSequence;

    #[test]
    fn test_get_minimum_sequence_empty() {
        let sequences: Vec<Arc<AtomicSequence>> = Vec::new();
        assert_eq!(Utils::get_minimum_sequence(&sequences), i64::MAX);
    }
}

pub struct AvailableSequenceBuffer {
    available_buffer: Vec<AtomicI64>,
    index_mask: i64,
}

impl AvailableSequenceBuffer {
    pub fn new(buffer_size: i64) -> Self {
        Self {
            available_buffer: (0..buffer_size).map(|_| AtomicI64::new(0)).collect(),
            index_mask: buffer_size - 1,
        }
    }

    pub fn set(&self, sequence: i64) {
        let index = sequence & self.index_mask;
        let flag = 1 << (sequence & 0x1f);
        unsafe {
            self.available_buffer
                .get_unchecked(index as usize)
                .fetch_or(flag, Ordering::SeqCst);
        }
    }

    pub fn is_set(&self, sequence: i64) -> bool {
        let index = sequence & self.index_mask;
        let flag = 1 << (sequence & 0x1f);
        unsafe {
            self.available_buffer
                .get_unchecked(index as usize)
                .load(Ordering::SeqCst)
                & flag
                != 0
        }
    }

    pub fn unset(&self, sequence: i64) {
        let index = sequence & self.index_mask;
        unsafe {
            self.available_buffer
                .get_unchecked(index as usize)
                .fetch_and(0, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_minimum_sequence() {
        let sequences = vec![
            Arc::new(AtomicSequence::new(1)),
            Arc::new(AtomicSequence::new(2)),
            Arc::new(AtomicSequence::new(3)),
        ];
        assert_eq!(Utils::get_minimum_sequence(&sequences), 1);

        // Test empty sequence
        let empty: Vec<Arc<AtomicSequence>> = vec![];
        assert_eq!(Utils::get_minimum_sequence(&empty), 0); // default value
    }

    #[test]
    fn test_get_maximum_sequence() {
        let sequences = vec![
            Arc::new(AtomicSequence::new(1)),
            Arc::new(AtomicSequence::new(2)),
            Arc::new(AtomicSequence::new(3)),
        ];
        assert_eq!(Utils::get_maximum_sequence(&sequences), 3);

        // Test empty sequence
        let empty: Vec<Arc<AtomicSequence>> = vec![];
        assert_eq!(Utils::get_maximum_sequence(&empty), 0); // default value
    }

    #[test]
    fn test_available_sequence_buffer() {
        let buffer_size = 1024;
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        // Test initial state, all slots are initialized to 0, so none are set.
        for i in 0..buffer_size {
            assert!(!buffer.is_set(i));
        }

        // Test setting even sequences.
        for i in 0..buffer_size {
            if i % 2 == 0 {
                buffer.set(i);
            }
        }

        // Test that even sequences are set and odd sequences are not.
        for i in 0..buffer_size {
            assert_eq!(buffer.is_set(i), i % 2 == 0);
        }
    }
}
