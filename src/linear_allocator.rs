use core::{
    alloc::GlobalAlloc,
    cell::UnsafeCell,
    ops::AddAssign,
    ptr,
    sync::atomic::{AtomicU8, AtomicUsize, Ordering},
};

const ARENA_SIZE: usize = 4096;
const MAX_SUPPORTED_ALIGN: usize = 4096;

pub struct LinearAllocator {
    arena: UnsafeCell<[u8; ARENA_SIZE]>,
    remaining: AtomicUsize,
}

unsafe impl GlobalAlloc for LinearAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();
        if align > MAX_SUPPORTED_ALIGN {
            return ptr::null_mut();
        }

        let base_ptr = self.arena.get() as *mut [u8; ARENA_SIZE];
        let mut ptr: *mut u8 = ptr::null_mut();
        if self
            .remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |mut remaining| {
                if ARENA_SIZE - remaining >= size {
                    return None;
                }

                let align_mask_to_round_down = !(align - 1);
                remaining -= size;
                remaining &= align_mask_to_round_down;

                ptr = unsafe { base_ptr.cast::<u8>().add(remaining) };
                Some(remaining)
            })
            .is_err()
        {
            return ptr::null_mut();
        };

        return ptr;
    }

    /// Deallocates the entire arena at once
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {
        self.remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |_| Some(ARENA_SIZE));
        let ptr = self.arena.get() as *mut [u8; ARENA_SIZE];
        unsafe {
            ptr.write([0; ARENA_SIZE]);
        }
    }

    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        todo!()
    }

    unsafe fn alloc_zeroed(&self, layout: core::alloc::Layout) -> *mut u8 {
        todo!()
    }
}
