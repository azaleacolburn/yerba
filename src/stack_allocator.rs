use core::{
    alloc::{self, GlobalAlloc},
    cell::UnsafeCell,
    ptr,
};
use std::{
    alloc::{AllocError, Allocator},
    ptr::NonNull,
};

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

    pub fn is_top(&self, ptr: *const u8, size: usize) -> bool {
        unsafe { ptr.addr() + size == self.buf.get().byte_add(self.get_offset()).addr() }
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

        Ok(ptr)
    }

    /// If ptr was not to the last allocated object, nothing happens
    unsafe fn dealloc(&self, ptr: *mut u8, layout: alloc::Layout) {
        let size = layout.size();
        if !self.is_top(ptr, layout.size()) {
            return;
            // panic!("Cannot deallocate block that is not on the top of the stack")
        }

        self.sub_offset(size);
    }

    unsafe fn alloc_zeroed(&self, layout: alloc::Layout) -> *mut u8 {
        let size = layout.size();
        let ptr = unsafe { self.alloc(layout) };

        unsafe {
            (0..size).for_each(|i| ptr.add(i).write(0));
        }

        ptr
    }

    /// Panics if the memory to be reallocated is not on the top of the stack
    /// Grows the allocated memory in-place
    unsafe fn realloc(&self, ptr: *mut u8, layout: alloc::Layout, new_size: usize) -> *mut u8 {
        let size = layout.size();
        let top = unsafe { self.buf.get().byte_add(self.get_offset()).addr() };
        assert_eq!(ptr.addr() + size, top);
        self.add_offset(new_size - size);

        ptr
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
            let chunk = allocator.alloc(layout);
            assert!(!chunk.is_null());
            allocator.dealloc(chunk, layout);

            let one = allocator.alloc(layout);
            assert!(!one.is_null());

            let two = allocator.alloc(layout);
            assert!(!two.is_null());

            let three = allocator.alloc(layout);
            assert!(!three.is_null());

            allocator.dealloc(three, layout);
            allocator.dealloc(two, layout);
            allocator.dealloc(one, layout);
        }
    }
    //
    // #[test]
    // #[should_panic]
    // fn out_of_order() {
    //     let allocator = StackAllocator::new();
    //     let layout = Layout::new::<[u8; 16]>();
    //
    //     unsafe {
    //         let one = allocator.alloc(layout);
    //         assert!(!one.is_null());
    //
    //         let two = allocator.alloc(layout);
    //         assert!(!two.is_null());
    //
    //         allocator.dealloc(one, layout);
    //     }
    // }

    #[test]
    fn zeroed() {
        let allocator = StackAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc_zeroed(layout);
            assert!(!one.is_null());

            let two = allocator.alloc_zeroed(layout);
            assert!(!two.is_null());

            allocator.dealloc(two, layout);
            allocator.dealloc(one, layout);
        }
    }

    #[test]
    fn realloc() {
        let allocator = StackAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc_zeroed(layout);
            assert!(!one.is_null());

            let two = allocator.alloc_zeroed(layout);
            assert!(!two.is_null());

            allocator.realloc(two, layout, 32);
            allocator.dealloc(two, Layout::new::<[u8; 32]>());
            allocator.dealloc(one, layout);
        }
    }
}
