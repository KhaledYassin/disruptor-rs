use std::cell::UnsafeCell;

use crate::{sequence::Sequence, traits::DataProvider};

/// A ring buffer with a fixed capacity.
/// The capacity must be a power of 2. The buffer is initialized with default values.
/// It is assumed that anything reading from or writing to this buffer will hold its own
/// index into the buffer, so the buffer itself does not keep track of the current index.
/// # Types
/// - `T`: The type of elements in the buffer. It must implement the `Default` and `Send` traits.
///   The `Default` trait is used to initialize the buffer with default values. The `Send` trait is
///   used to allow the data to be sent between threads.  
/// # Safety
/// We require intrior mutability to allow for multiple readers and writers to access the buffer concurrently.
pub struct RingBuffer<T> {
    capacity: usize,
    _mask: usize,
    _data: Vec<UnsafeCell<T>>,
}

const fn is_power_of_two(x: usize) -> bool {
    x != 0 && (x & (x - 1)) == 0
}

unsafe impl<T: Send> Send for RingBuffer<T> {}
unsafe impl<T: Sync> Sync for RingBuffer<T> {}

impl<T: Default + Send> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(is_power_of_two(capacity), "Capacity must be a power of 2");
        Self {
            capacity,
            _mask: capacity - 1,
            _data: (0..capacity)
                .map(|_| UnsafeCell::new(T::default()))
                .collect(), // Initialize buffer with default values
        }
    }

    pub fn get_capacity(&self) -> usize {
        self.capacity
    }
}

impl<T: Send + Sync> DataProvider<T> for RingBuffer<T> {
    fn get_capacity(&self) -> usize {
        self.capacity
    }

    /// Get a reference to the element at the given sequence.
    /// # Safety
    /// This method is unsafe because it allows for multiple readers to access the buffer concurrently.
    /// The caller must ensure that the sequence is within the bounds of the buffer.
    /// # Arguments
    /// - `sequence`: The sequence of the element to get.
    /// # Returns
    /// A reference to the element at the given sequence.
    unsafe fn get(&self, sequence: Sequence) -> &T {
        let index = sequence as usize & self._mask;
        &*self._data[index].get()
    }


    /// Get a mutable reference to the element at the given sequence.
    /// # Safety
    /// This method is unsafe because it allows for multiple writers to access the buffer concurrently.
    /// The caller must ensure that the sequence is within the bounds of the buffer.
    /// # Arguments
    /// - `sequence`: The sequence of the element to get.
    /// # Returns
    /// A mutable reference to the element at the given sequence.
    unsafe fn get_mut(&self, sequence: Sequence) -> &mut T {
        let index = sequence as usize & self._mask;
        &mut *self._data[index].get()
    }
}

#[cfg(test)]
mod tests {

    use std::{sync::Arc, thread};

    use super::*;

    const ITERATIONS: i64 = 256;
    const THREADS: usize = 4;

    #[test]
    fn test_initialization() {
        let buffer = RingBuffer::<i64>::new(ITERATIONS as usize);
        
        assert_eq!(buffer.get_capacity(), 256);

        for i in 0..ITERATIONS {
            unsafe {
                assert_eq!(*buffer.get(i), 0);
            }
        }
    }

    #[test]
    fn test_ring_buffer() {
        let buffer = RingBuffer::<i64>::new(ITERATIONS as usize);
        assert_eq!(buffer.get_capacity(), 256);

        for i in 0..ITERATIONS {
            unsafe {
                *buffer.get_mut(i) = i;
            }
        }

        for i in 0..ITERATIONS {
            unsafe {
                *buffer.get_mut(i) *= 2;
            }
        }

        for i in 0..ITERATIONS {
            unsafe {
                assert_eq!(*buffer.get(i), i * 2);
            }
        }
    }

    #[test]
    fn test_ring_buffer_multithreaded() {
        let buffer = Arc::new(RingBuffer::<i64>::new(ITERATIONS as usize));
        let mut handles = vec![];

        for _ in 0..THREADS {
            let buffer = buffer.clone();
            let handle = thread::spawn(move || {
                for i in 0..ITERATIONS {
                    unsafe {
                        *buffer.get_mut(i) += i;
                    }
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        for i in 0..ITERATIONS {
            unsafe {
                assert_eq!(*buffer.get(i), i * THREADS as i64);
            }
        }
    }
}
