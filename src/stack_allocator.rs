use crate::util::to_non_null_slice;
use core::{
    alloc::{AllocError, Allocator, Layout},
    cell::UnsafeCell,
    ptr::NonNull,
};

const BUF_SIZE: usize = (4096 * 2) / (size_of::<usize>() / size_of::<u8>()) + size_of::<usize>();

/// Allocator shaped like a stack
/// Allows the allocation and deallocation of memory in a LIFO system
/// Allocates an initial buffer of 4096 bytes in-place on the stack
///
/// # Use Cases
/// ## Use if
/// - You need blazingly fast LIFO allocations, in a loop, for instance
/// ## Limitations
/// - Fixed-size buffer, does not reserve more memory
/// - Can only perform LIFO allocations and deallocations
/// - Cannot perform reallocations
///
/// # Usage
///
/// ## With manual allocations
/// ```rust
/// #![feature(allocator_api)]
/// use yerba::stack_allocator::StackAllocator;
/// use core::alloc::{Allocator, Layout};
///
/// let allocator = StackAllocator::new();
/// let layout = Layout::new::<[u8; 16]>();
///
/// unsafe {
///     let chunk = allocator.allocate(layout).unwrap().cast();
///     allocator.deallocate(chunk, layout);
///
///     let one = allocator.allocate(layout).unwrap().cast();
///     let two = allocator.allocate(layout).unwrap().cast();
///     let three = allocator.allocate(layout).unwrap().cast();
///
///     allocator.deallocate(three, layout);
///     allocator.deallocate(two, layout);
///     allocator.deallocate(one, layout);
/// }
/// ```
/// ## With Rust structures
///
/// ```rust
/// #![feature(allocator_api)]
/// use yerba::stack_allocator::StackAllocator;
/// use core::alloc::{Allocator, Layout};
///
///
/// let allocator = StackAllocator::new();
/// // Be careful! This example works because objects are dropped in the [reverse order of
/// // their declaration](https://doc.rust-lang.org/reference/destructors.html), so this example obeys
/// // LIFO rules
/// for i in 0..12 {
///     let mut one = Box::new_in([0; 16], &allocator);
///     let mut two = Box::new_in([0 as i32; 13], &allocator);
///     let mut three = Box::new_in([0 as usize; 12], &allocator);
///
///     one[i] = 14;
///     two[12] = 9;
///     three[4] = one.as_ptr().addr();
///
///     // Gets dropped automatically in the correct order
/// }
/// ```
///
/// # Notes
/// - Future iterations (or future allocators) may be LIFO and allow buffer growth as well as
///   custom initial buffer sizes
///
/// # Safety
/// - Calling `StackAllocator::deallocate` in an out of order manner will not panic, it will cause
///   a silent failure
pub struct StackAllocator {
    buf: UnsafeCell<[usize; BUF_SIZE]>,
}

impl StackAllocator {
    #[must_use]
    pub const fn new() -> Self {
        let mut buf = UnsafeCell::new([0; BUF_SIZE]);

        let ptr = buf.get_mut().as_mut_ptr().cast::<usize>();
        unsafe { ptr.write(size_of::<usize>()) };

        Self { buf }
    }

    #[inline]
    pub const fn get_offset(&self) -> usize {
        unsafe { (self.buf.get().cast::<usize>()).read() }
    }

    #[inline]
    pub const fn set_offset(&mut self, n: usize) {
        unsafe { (self.buf.get_mut().as_mut_ptr().cast::<usize>()).write(n) }
    }

    pub const fn add_offset(&self, n: usize) {
        unsafe {
            let ptr = self.buf.get().cast::<usize>();
            let value = ptr.read();
            ptr.write(value + n);
        }
    }

    pub const fn sub_offset(&self, n: usize) {
        unsafe {
            let ptr = self.buf.get().cast::<usize>();
            let value = ptr.read();
            ptr.write(value - n);
        }
    }

    #[inline]
    pub fn is_top(&self, ptr: NonNull<u8>, size: usize) -> bool {
        unsafe {
            usize::from(ptr.addr()) + size == self.buf.get().byte_add(self.get_offset()).addr()
        }
    }
}

impl Default for StackAllocator {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Allocator for StackAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();
        let align = layout.align();
        let buf_offset = self.get_offset();

        let mut ptr: *mut u8 = unsafe { self.buf.get().byte_add(buf_offset) }.cast();

        let alignment_offset = ptr.align_offset(align);
        if alignment_offset == usize::MAX {
            return Err(AllocError);
        }
        ptr = unsafe { ptr.add(alignment_offset) };

        let last_addr = unsafe { self.buf.get().byte_add(BUF_SIZE).addr() };
        if ptr.addr() + size >= last_addr {
            return Err(AllocError);
        }

        self.add_offset(size);

        to_non_null_slice(ptr, size)
    }

    /// If ptr was not to the last allocated object, nothing happens
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let size = layout.size();
        if !self.is_top(ptr, size) {
            return;
        }

        self.sub_offset(size);
    }

    /// Panics if the memory to be reallocated is not on the top of the stack
    /// Grows the allocated memory in-place
    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();
        let new_size = new_layout.size();
        let top = unsafe { self.buf.get().byte_add(self.get_offset()).addr() };
        assert_eq!(usize::from(ptr.addr()) + size, top);
        self.add_offset(new_size - size);

        Ok(NonNull::slice_from_raw_parts(ptr, new_size))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use core::alloc::Layout;
    use std::boxed::Box;

    #[test]
    fn alloc_chunks() {
        let allocator = StackAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let chunk = allocator.allocate(layout).unwrap().cast();
            allocator.deallocate(chunk, layout);

            let one = allocator.allocate(layout).unwrap().cast();
            let two = allocator.allocate(layout).unwrap().cast();
            let three = allocator.allocate(layout).unwrap().cast();

            allocator.deallocate(three, layout);
            allocator.deallocate(two, layout);
            allocator.deallocate(one, layout);
        }
    }

    #[test]
    fn zeroed() {
        let allocator = StackAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.allocate_zeroed(layout).unwrap().cast();
            let two = allocator.allocate_zeroed(layout).unwrap().cast();

            allocator.deallocate(two, layout);
            allocator.deallocate(one, layout);
        }
    }

    #[test]
    fn realloc() {
        let allocator = StackAllocator::new();
        let layout = Layout::new::<[u8; 16]>();
        let second_layout = Layout::new::<[u8; 32]>();

        unsafe {
            let one = allocator.allocate_zeroed(layout).unwrap().cast();
            let two = allocator.allocate_zeroed(layout).unwrap().cast();

            allocator.grow(two, layout, second_layout);
            allocator.deallocate(two, second_layout);
            allocator.deallocate(one, layout);
        }
    }

    #[test]
    fn in_loop() {
        let allocator = StackAllocator::new();

        for i in 0..12 {
            let mut one = Box::new_in([0; 16], &allocator);
            let mut two = Box::new_in([0 as i32; 13], &allocator);
            let mut three = Box::new_in([0 as usize; 12], &allocator);

            one[i] = 14;
            two[12] = 9;
            three[4] = one.as_ptr().addr();

            // Gets dropped automatically
        }
    }
}
