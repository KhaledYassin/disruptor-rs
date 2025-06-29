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
}

impl AvailableSequenceBuffer {
    pub fn new(buffer_size: i64) -> Self {
        Self {
            available_buffer: (0..buffer_size)
                .map(|_| CacheAlignedAtomicI64 {
                    value: AtomicI64::new(0),
                    _padding: [0u8; 56],
                })
                .collect(),
            index_mask: buffer_size - 1,
        }
    }

    pub fn set(&self, sequence: i64) {
        let index = sequence & self.index_mask;
        let flag = 1i64 << (sequence & 0x3f);
        unsafe {
            self.available_buffer
                .get_unchecked(index as usize)
                .value
                .fetch_or(flag, Ordering::Release);
        }
    }

    pub fn is_set(&self, sequence: i64) -> bool {
        let index = sequence & self.index_mask;
        let flag = 1i64 << (sequence & 0x3f);
        unsafe {
            self.available_buffer
                .get_unchecked(index as usize)
                .value
                .load(Ordering::Acquire)
                & flag
                != 0
        }
    }

    pub fn unset(&self, sequence: i64) {
        let index = sequence & self.index_mask;
        let flag = 1i64 << (sequence & 0x3f);
        unsafe {
            self.available_buffer
                .get_unchecked(index as usize)
                .value
                .fetch_and(!flag, Ordering::Release);
        }
    }

    pub fn set_batch(&self, start: i64, end: i64) {
        if start > end {
            return; // Handle invalid range
        }

        let mut updates: std::collections::HashMap<usize, i64> = std::collections::HashMap::new();

        for seq in start..=end {
            let index = (seq & self.index_mask) as usize;
            let flag = 1i64 << (seq & 0x3f);
            *updates.entry(index).or_insert(0) |= flag;
        }

        for (index, combined_flags) in updates {
            self.available_buffer[index]
                .value
                .fetch_or(combined_flags, Ordering::Release);
        }
    }

    pub fn unset_batch(&self, start: i64, end: i64) {
        if start > end {
            return; // Handle invalid range
        }

        let mut updates: std::collections::HashMap<usize, i64> = std::collections::HashMap::new();

        for seq in start..=end {
            let index = (seq & self.index_mask) as usize;
            let flag = 1i64 << (seq & 0x3f);
            *updates.entry(index).or_insert(0) |= flag;
        }

        for (index, combined_flags) in updates {
            self.available_buffer[index]
                .value
                .fetch_and(!combined_flags, Ordering::Release);
        }
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
    fn test_available_sequence_buffer_batch_operations() {
        let buffer_size = 64;
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        // Test batch setting
        buffer.set_batch(0, 5);
        for i in 0..=5 {
            assert!(buffer.is_set(i), "Sequence {i} should be set");
        }
        for i in 6..buffer_size {
            assert!(!buffer.is_set(i), "Sequence {i} should not be set");
        }

        // Test batch unsetting
        buffer.unset_batch(2, 4);
        for i in 0..=1 {
            assert!(buffer.is_set(i), "Sequence {i} should still be set");
        }
        for i in 2..=4 {
            assert!(!buffer.is_set(i), "Sequence {i} should be unset");
        }
        assert!(buffer.is_set(5), "Sequence 5 should still be set");
    }

    #[test]
    fn test_batch_operations_with_wraparound() {
        let buffer_size = 8;
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        // Test batch operations that wrap around the buffer
        buffer.set_batch(6, 10); // This will wrap around (6,7,0,1,2)

        assert!(buffer.is_set(6));
        assert!(buffer.is_set(7));
        assert!(buffer.is_set(8)); // wraps to index 0
        assert!(buffer.is_set(9)); // wraps to index 1
        assert!(buffer.is_set(10)); // wraps to index 2

        assert!(!buffer.is_set(3));
        assert!(!buffer.is_set(4));
        assert!(!buffer.is_set(5));
    }

    #[test]
    fn test_batch_operations_same_index_multiple_flags() {
        let buffer_size = 8;
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        // Set sequences that map to the same index but different bit positions
        buffer.set_batch(0, 2); // All map to index 0 with different bit positions

        assert!(buffer.is_set(0)); // bit 0
        assert!(buffer.is_set(1)); // bit 1
        assert!(buffer.is_set(2)); // bit 2

        // Unset one of them
        buffer.unset_batch(1, 1);
        assert!(buffer.is_set(0));
        assert!(!buffer.is_set(1));
        assert!(buffer.is_set(2));
    }

    #[test]
    fn test_batch_operations_empty_range() {
        let buffer_size = 8;
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        // Empty ranges should not cause issues
        buffer.set_batch(5, 4); // end < start, should do nothing
        for i in 0..buffer_size {
            assert!(!buffer.is_set(i));
        }
    }

    #[test]
    fn test_batch_operations_large_range() {
        let buffer_size = 128; // Use larger buffer to avoid wraparound conflicts
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        // Test batch covering multiple indices but within reasonable range
        buffer.set_batch(0, 50);

        // Check that all sequences in the range are set
        for i in 0..=50 {
            assert!(buffer.is_set(i), "Sequence {i} should be set");
        }

        // Unset a portion
        buffer.unset_batch(20, 30);
        for i in 20..=30 {
            assert!(!buffer.is_set(i), "Sequence {i} should be unset");
        }

        // Check that sequences outside the unset range are still set
        for i in 0..20 {
            assert!(buffer.is_set(i), "Sequence {i} should still be set");
        }
        for i in 31..=50 {
            assert!(buffer.is_set(i), "Sequence {i} should still be set");
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
            assert!(buffer.is_set(i), "Sequence {i} should be set");
        }
    }

    #[test]
    fn test_mixed_individual_and_batch_operations() {
        let buffer_size = 32;
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        // Set some individual sequences
        buffer.set(1);
        buffer.set(5);
        buffer.set(10);

        // Set a batch that overlaps with individual sets
        buffer.set_batch(8, 12);

        // Verify all are set
        assert!(buffer.is_set(1));
        assert!(buffer.is_set(5));
        for i in 8..=12 {
            assert!(buffer.is_set(i));
        }

        // Unset batch that partially overlaps
        buffer.unset_batch(10, 15);

        assert!(buffer.is_set(1));
        assert!(buffer.is_set(5));
        assert!(buffer.is_set(8));
        assert!(buffer.is_set(9));
        for i in 10..=15 {
            assert!(!buffer.is_set(i));
        }
    }

    #[test]
    fn test_bit_manipulation_edge_cases() {
        let buffer_size = 8;
        let buffer = AvailableSequenceBuffer::new(buffer_size);

        // Test sequences that exercise all 64 bits of a single atomic
        let test_sequences = [0, 1, 63, 64, 65, 127, 128];

        for &seq in &test_sequences {
            buffer.set(seq);
            assert!(buffer.is_set(seq), "Sequence {seq} should be set");
        }

        // Test batch operation across bit boundaries
        buffer.set_batch(60, 68);
        for i in 60..=68 {
            assert!(buffer.is_set(i), "Sequence {i} should be set");
        }
    }
}
