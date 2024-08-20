use std::sync::atomic::{AtomicI64, Ordering};

// A sequence is a counter that can be incremented atomically.
// It is used to keep track of the current position in the ring buffer.
// The sequence is padded to avoid false sharing.

pub type Sequence = i64;

const CACHE_LINE_SIZE: usize = 64;
const CACHE_LINE_PADDING: usize = CACHE_LINE_SIZE - std::mem::size_of::<AtomicI64>();

#[repr(align(64))]
#[derive(Debug)]
pub struct AtomicSequence {
    value: AtomicI64,
    _padding: [u8; CACHE_LINE_PADDING],
}

impl AtomicSequence {
    // Create a new Sequence with an initial value.
    pub fn new(initial_value: Sequence) -> Self {
        AtomicSequence {
            value: AtomicI64::new(initial_value),
            _padding: [0u8; CACHE_LINE_PADDING],
        }
    }

    // Get the current value of the sequence.
    pub fn get(&self) -> Sequence {
        self.value.load(Ordering::SeqCst)
    }

    // Set a new value for the sequence.
    pub fn set(&self, new_value: Sequence) {
        self.value.store(new_value, Ordering::SeqCst);
    }

    // Increment the sequence by 1 and return the new value.
    pub fn get_and_increment(&self) -> Sequence {
        self.value.fetch_add(1, Ordering::SeqCst)
    }

    // Increment the sequence by 1 and return the old value.
    pub fn increment_and_get(&self) -> Sequence {
        self.value.fetch_add(1, Ordering::SeqCst) + 1
    }
}

impl Default for AtomicSequence {
    fn default() -> Self {
        Self::new(-1)
    }
}

impl PartialEq for AtomicSequence {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn test_sequence() {
        let sequence = AtomicSequence::new(0);
        assert_eq!(sequence.get(), 0);
        assert_eq!(sequence.get_and_increment(), 0);
        assert_eq!(sequence.get(), 1);
        assert_eq!(sequence.increment_and_get(), 2);
        assert_eq!(sequence.get(), 2);
        sequence.set(42);
        assert_eq!(sequence.get(), 42);
    }

    #[test]
    fn test_sequence_concurrent() {
        let sequence = AtomicSequence::new(0);
        let mut handles = vec![];
        let sequence_arc = Arc::new(sequence);
        for _ in 0..10 {
            let sequence = sequence_arc.clone();
            let handle = std::thread::spawn(move || {
                for _ in 0..1000 {
                    sequence.get_and_increment();
                }
            });
            handles.push(handle);
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(sequence_arc.get(), 10000);
    }

    #[test]
    fn test_sequence_concurrent_increment_and_get() {
        let sequence = AtomicSequence::new(0);
        let mut handles = vec![];
        let sequence_arc = Arc::new(sequence);
        for _ in 0..10 {
            let sequence = sequence_arc.clone();
            let handle = std::thread::spawn(move || {
                for _ in 0..1000 {
                    sequence.increment_and_get();
                }
            });
            handles.push(handle);
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(sequence_arc.get(), 10000);
    }
}
