use core::{
    alloc::{self, GlobalAlloc},
    cell::UnsafeCell,
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

const BUF_SIZE: usize = 4096;

/// Allows the allocation and deallocation of memory in a LIFO system
/// Allocates an initial buffer of 4096 bytes
pub struct StackAllocator {
    buf: UnsafeCell<[u8; BUF_SIZE]>,
    offset: AtomicUsize,
}

impl StackAllocator {
    pub fn new() -> Self {
        StackAllocator {
            buf: UnsafeCell::new([0; BUF_SIZE]),
            offset: AtomicUsize::new(0),
        }
    }

    pub fn is_top(&self, ptr: *const u8, size: usize) -> bool {
        unsafe {
            ptr.addr() + size
                == self
                    .buf
                    .get()
                    .byte_add(self.offset.load(Ordering::Relaxed))
                    .addr()
        }
    }

    fn assert_top(&self, ptr: *const u8, size: usize) {
        unsafe {
            assert_eq!(
                ptr.addr() + size,
                self.buf
                    .get()
                    .byte_add(self.offset.load(Ordering::Relaxed))
                    .addr()
            )
        }
    }
}

unsafe impl GlobalAlloc for StackAllocator {
    unsafe fn alloc(&self, layout: alloc::Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();
        let buf_offset = self.offset.load(Ordering::Relaxed);

        let mut ptr: *mut u8 = unsafe { self.buf.get().byte_add(buf_offset) }.cast();

        let alignment_offset = ptr.align_offset(align);
        if alignment_offset == usize::MAX {
            return ptr::null_mut();
        }
        ptr = unsafe { ptr.add(alignment_offset) };

        if ptr.addr() + size >= unsafe { self.buf.get().byte_add(BUF_SIZE).addr() } {
            return ptr::null_mut();
        }

        self.offset
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |offset| {
                Some(offset + size)
            })
            .unwrap();

        ptr
    }

    /// Panics if ptr was not the last allocated object
    unsafe fn dealloc(&self, ptr: *mut u8, layout: alloc::Layout) {
        self.offset
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |offset| {
                let size = layout.size();
                self.assert_top(ptr, size);

                Some(offset - size)
            })
            .unwrap();
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
        let top = unsafe {
            self.buf
                .get()
                .byte_add(self.offset.load(Ordering::Relaxed))
                .addr()
        };
        assert_eq!(ptr.addr() + size, top);
        self.offset.fetch_add(new_size - size, Ordering::Relaxed);

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

    #[test]
    #[should_panic]
    fn out_of_order() {
        let allocator = StackAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc(layout);
            assert!(!one.is_null());

            let two = allocator.alloc(layout);
            assert!(!two.is_null());

            allocator.dealloc(one, layout);
        }
    }

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
