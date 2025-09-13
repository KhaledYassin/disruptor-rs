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

#[repr(align(64))]
struct CacheAlignedAtomicI64 {
    value: AtomicI64,
    _padding: [u8; 56],
}

pub struct AvailableSequenceBuffer {
    available_buffer: Vec<CacheAlignedAtomicI64>,
    index_mask: i64,
    index_shift: u32,
}

impl AvailableSequenceBuffer {
    pub fn new(buffer_size: i64) -> Self {
        assert!(
            buffer_size > 0 && (buffer_size as u64).is_power_of_two(),
            "AvailableSequenceBuffer size must be power-of-two"
        );
        let index_shift = (buffer_size as usize).trailing_zeros();
        Self {
            available_buffer: (0..buffer_size)
                .map(|_| CacheAlignedAtomicI64 {
                    value: AtomicI64::new(-1),
                    _padding: [0u8; 56],
                })
                .collect(),
            index_mask: buffer_size - 1,
            index_shift,
        }
    }

    #[inline]
    pub fn set(&self, sequence: i64) {
        let index = (sequence & self.index_mask) as usize;
        let generation = sequence >> self.index_shift;
        unsafe {
            self.available_buffer
                .get_unchecked(index)
                .value
                .store(generation, Ordering::Release);
        }
    }

    #[inline]
    pub fn is_available(&self, sequence: i64) -> bool {
        let index = (sequence & self.index_mask) as usize;
        let expected_generation = sequence >> self.index_shift;
        let observed = unsafe {
            self.available_buffer
                .get_unchecked(index)
                .value
                .load(Ordering::Acquire)
        };
        observed == expected_generation
    }

    // No longer needed in generation-based design: unset is removed

    pub fn set_batch(&self, start: i64, end: i64) {
        if start > end {
            return; // Handle invalid range
        }

        // Optimize for small batches (common case) by inlining the set logic
        let batch_size = end - start + 1;
        
        if batch_size <= 8 {
            // Small batch: inline the set operations to reduce function call overhead
            for seq in start..=end {
                let index = (seq & self.index_mask) as usize;
                let generation = seq >> self.index_shift;
                unsafe {
                    self.available_buffer
                        .get_unchecked(index)
                        .value
                        .store(generation, Ordering::Release);
                }
            }
        } else {
            // Large batch: check for wraparound and optimize accordingly
            let start_index = (start & self.index_mask) as usize;
            let end_index = (end & self.index_mask) as usize;
            
            if start_index <= end_index {
                // No wraparound case - can process sequentially with better cache locality
                let start_gen = start >> self.index_shift;
                for (i, seq) in (start..=end).enumerate() {
                    let index = start_index + i;
                    let generation = start_gen + ((seq - start) >> self.index_shift);
                    unsafe {
                        self.available_buffer
                            .get_unchecked(index)
                            .value
                            .store(generation, Ordering::Release);
                    }
                }
            } else {
                // Wraparound case - fall back to individual calls for correctness
                for seq in start..=end {
                    self.set(seq);
                }
            }
        }
    }

    // No longer needed in generation-based design: unset_batch is removed

    /// Returns the highest contiguous published sequence in [from, to].
    /// If `from` is not yet available, returns `from - 1`.
    pub fn highest_published_sequence(&self, from: i64, to: i64) -> i64 {
        if from > to {
            return from - 1;
        }

        let mut last = from - 1;
        let mut seq = from;
        while seq <= to {
            if self.is_available(seq) {
                last = seq;
                seq += 1;
            } else {
                break;
            }
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::AtomicSequence;
    use std::sync::Arc;
    use std::thread;

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
        assert_eq!(Utils::get_minimum_sequence(&empty), i64::MAX);
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
        assert_eq!(Utils::get_maximum_sequence(&empty), i64::MIN);
    }

    #[test]
    fn test_cache_aligned_atomic_size_and_alignment() {
        // Verify that CacheAlignedAtomicI64 is properly sized and aligned
        assert_eq!(std::mem::size_of::<CacheAlignedAtomicI64>(), 64);
        assert_eq!(std::mem::align_of::<CacheAlignedAtomicI64>(), 64);

        // Verify that the struct is cache-line aligned
        let aligned_atomic = CacheAlignedAtomicI64 {
            value: AtomicI64::new(42),
            _padding: [0u8; 56],
        };
        let addr = &aligned_atomic as *const _ as usize;
        assert_eq!(
            addr % 64,
            0,
            "CacheAlignedAtomicI64 should be 64-byte aligned"
        );
    }

    #[test]
    fn test_available_sequence_buffer_generation_basic() {
        let buffer_size = 64;
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        buffer.set_batch(0, 5);
        for i in 0..=5 {
            assert!(buffer.is_available(i), "Sequence {i} should be available");
        }
        for i in 6..buffer_size {
            assert!(
                !buffer.is_available(i),
                "Sequence {i} should not be available"
            );
        }
    }

    #[test]
    #[should_panic]
    fn test_available_buffer_rejects_non_power_of_two() {
        let _ = AvailableSequenceBuffer::new(12); // not a power of two
    }

    #[test]
    fn test_batch_operations_with_wraparound() {
        let buffer_size = 8;
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        // Test batch operations that wrap around the buffer
        buffer.set_batch(6, 10); // This will wrap around indices (6,7,0,1,2)

        assert!(buffer.is_available(6));
        assert!(buffer.is_available(7));
        assert!(buffer.is_available(8)); // wraps to index 0 with next generation
        assert!(buffer.is_available(9)); // wraps to index 1 with next generation
        assert!(buffer.is_available(10)); // wraps to index 2 with next generation

        assert!(!buffer.is_available(3));
        assert!(!buffer.is_available(4));
        assert!(!buffer.is_available(5));
    }

    #[test]
    fn test_batch_operations_empty_range() {
        let buffer_size = 8;
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        // Empty ranges should not cause issues
        buffer.set_batch(5, 4); // end < start, should do nothing
        for i in 0..buffer_size {
            assert!(!buffer.is_available(i));
        }
    }

    #[test]
    fn test_concurrent_batch_operations() {
        let buffer_size = 64;
        let buffer = Arc::new(AvailableSequenceBuffer::new(buffer_size));
        let num_threads = 4;
        let sequences_per_thread = 16;

        let mut handles = vec![];

        // Spawn threads that set different ranges concurrently
        for thread_id in 0..num_threads {
            let buffer_clone = Arc::clone(&buffer);
            let handle = thread::spawn(move || {
                let start = thread_id as i64 * sequences_per_thread;
                let end = start + sequences_per_thread - 1;
                buffer_clone.set_batch(start, end);
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify all sequences were set correctly
        for i in 0..(num_threads as i64 * sequences_per_thread) {
            assert!(buffer.is_available(i), "Sequence {i} should be available");
        }
    }

    #[test]
    fn test_generation_overwrite_on_wrap() {
        // With size 8, index_shift = 3. Publishing 0 replaces gen=0 at index 0,
        // publishing 8 replaces with gen=1 at index 0.
        let buffer = AvailableSequenceBuffer::new(8);
        buffer.set(0);
        assert!(buffer.is_available(0));
        assert!(!buffer.is_available(8));

        buffer.set(8);
        assert!(buffer.is_available(8));
        assert!(!buffer.is_available(0));
    }

    #[test]
    fn test_highest_published_sequence() {
        let buffer = AvailableSequenceBuffer::new(16);

        // Nothing published
        assert_eq!(buffer.highest_published_sequence(0, 10), -1);

        // Publish 0 and 2 (gap at 1)
        buffer.set(0);
        buffer.set(2);
        assert_eq!(buffer.highest_published_sequence(0, 5), 0);

        // Fill the gap
        buffer.set(1);
        assert_eq!(buffer.highest_published_sequence(0, 5), 2);

        // From mid-range
        assert_eq!(buffer.highest_published_sequence(1, 5), 2);

        // from > to
        assert_eq!(buffer.highest_published_sequence(5, 4), 4);
    }
}
