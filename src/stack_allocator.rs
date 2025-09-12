use core::{
    alloc::GlobalAlloc,
    cell::UnsafeCell,
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

use libc::uintptr_t;

const BUF_SIZE: usize = 4096;

/// Allows the allocation and deallocation of memory in a LIFO system
struct StackAllocator {
    buf: UnsafeCell<[u8; BUF_SIZE]>,
    offset: AtomicUsize,
}

unsafe impl GlobalAlloc for StackAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();
        let buf_offset = self.offset.load(Ordering::Relaxed);

        let mut ptr: *mut u8 = unsafe { self.buf.get().add(buf_offset) }.cast();

        let alignment_offset = ptr.align_offset(align);
        if alignment_offset == usize::MAX {
            return ptr::null_mut();
        }
        ptr = unsafe { ptr.add(alignment_offset) };

        if ptr.addr() >= BUF_SIZE {
            return ptr::null_mut();
        }

        self.offset
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |_| {
                Some(ptr.addr() + size)
            });

        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        self.offset
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |offset| {
                let size = layout.size();
                let ptr_addr = ptr.addr();
                assert!(ptr_addr + size == offset);

                Some(ptr_addr)
            });
    }

    unsafe fn alloc_zeroed(&self, layout: core::alloc::Layout) -> *mut u8 {}

    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
    }
}

fn align_forward(mut ptr: &mut *mut u8, align: usize) {
    assert!(align.is_power_of_two());

    let modulo = ptr.into() & (align - 1);

    if modulo != 0 {
        ptr.align_offset.add_assign(align - modulo);
    }
}
