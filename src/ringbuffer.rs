use std::cell::UnsafeCell;

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
    mask: usize,
    buffer: UnsafeCell<Vec<T>>,
}

const fn is_power_of_two(x: usize) -> bool {
    x != 0 && (x & (x - 1)) == 0
}

impl<T: Default + Send> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(is_power_of_two(capacity), "Capacity must be a power of 2");
        Self {
            capacity,
            mask: capacity - 1,
            buffer: UnsafeCell::new((0..capacity).map(|_| T::default()).collect()),
        }
    }

    pub fn get_capacity(&self) -> usize {
        self.capacity
    }

    pub fn write(&mut self, index: usize, element: T) {
        self.buffer.get_mut()[index & self.mask] = element;
    }

    pub unsafe fn read(&mut self, index: usize) -> &T {
        &self.buffer.get().as_ref().unwrap()[index & self.mask]
    }
}

#[cfg(test)]
mod tests {

    use std::marker::PhantomData;

    use super::*;

    #[test]
    fn test_ring_buffer() {
        unsafe {
            let mut ring_buffer = RingBuffer::new(4);
            ring_buffer.write(0, 1);
            ring_buffer.write(1, 2);
            ring_buffer.write(2, 3);
            ring_buffer.write(3, 4);
            assert_eq!(*ring_buffer.read(0), 1);
            assert_eq!(*ring_buffer.read(1), 2);
            assert_eq!(*ring_buffer.read(2), 3);
            assert_eq!(*ring_buffer.read(3), 4);
            ring_buffer.write(4, 5);
            assert_eq!(*ring_buffer.read(0), 5);
            assert_eq!(*ring_buffer.read(1), 2);
            assert_eq!(*ring_buffer.read(2), 3);
            assert_eq!(*ring_buffer.read(3), 4);
        }
    }

    #[test]
    #[should_panic]
    fn test_ring_buffer_capacity_not_power_of_two() {
        let _ring_buffer = RingBuffer::<PhantomData<i32>>::new(3);
    }
}
