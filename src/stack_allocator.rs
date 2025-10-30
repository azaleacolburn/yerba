use core::{
    alloc::{self},
    cell::UnsafeCell,
};
use std::{
    alloc::{AllocError, Allocator, Layout},
    ptr::NonNull,
};

use crate::util::to_non_null_slice;

const BUF_SIZE: usize = 4096;

/// Allows the allocation and deallocation of memory in a LIFO system
/// Allocates an initial buffer of 4096 bytes
pub struct StackAllocator {
    buf: UnsafeCell<[u8; BUF_SIZE]>,
    offset: *mut usize,
}

impl StackAllocator {
    pub fn new() -> Self {
        let mut buf = UnsafeCell::new([0; BUF_SIZE]);
        let ptr = buf.get_mut() as *mut [u8; BUF_SIZE] as *mut usize;
        unsafe { ptr.write(size_of::<usize>()) };
        StackAllocator { buf, offset: ptr }
    }

    pub fn get_offset(&self) -> usize {
        unsafe { (self.buf.get() as *mut usize).read() }
    }

    pub fn set_offset(&mut self, n: usize) {
        unsafe { (self.buf.get_mut() as *mut [u8; BUF_SIZE] as *mut usize).write(n) }
    }

    pub fn add_offset(&self, n: usize) {
        unsafe {
            let ptr = self.buf.get() as *mut usize;
            let value = ptr.read();
            ptr.write(value + n);
        }
    }

    pub fn sub_offset(&self, n: usize) {
        unsafe {
            let ptr = self.buf.get() as *mut usize;
            let value = ptr.read();
            ptr.write(value - n);
        }
    }

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
    fn allocate(&self, layout: alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();
        let align = layout.align();
        let buf_offset = self.get_offset();

        let mut ptr: *mut u8 = unsafe { self.buf.get().byte_add(buf_offset) }.cast();

        let alignment_offset = ptr.align_offset(align);
        if alignment_offset == usize::MAX {
            return Err(AllocError);
        }
        ptr = unsafe { ptr.add(alignment_offset) };

        if ptr.addr() + size >= unsafe { self.buf.get().byte_add(BUF_SIZE).addr() } {
            return Err(AllocError);
        }

        self.add_offset(size);

        Ok(to_non_null_slice(ptr, size)?)
    }

    /// If ptr was not to the last allocated object, nothing happens
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: alloc::Layout) {
        let size = layout.size();
        if !self.is_top(ptr, layout.size()) {
            return;
            // panic!("Cannot deallocate block that is not on the top of the stack")
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
    use core::alloc::Layout;

    use super::*;

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
}
