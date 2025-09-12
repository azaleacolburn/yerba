use core::{alloc::GlobalAlloc, cell::UnsafeCell, ops::AddAssign, ptr, sync::atomic::AtomicU8};

const ARENA_SIZE: usize = 4096;

pub struct LinearAllocator {
    arena: UnsafeCell<[u8; ARENA_SIZE]>,
    last: AtomicU8,
}

unsafe impl GlobalAlloc for LinearAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size();

        if ARENA_SIZE - self.last.into() >= size {
            let base_ptr = self.arena.get_mut() as *mut [u8; ARENA_SIZE];
            let ptr = unsafe { base_ptr.add(self.last.into()) };
            self.last.get_mut().add_assign(size.into());

            return ptr.cast::<u8>();
        }

        return ptr::null_mut();
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {}

    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
    }

    unsafe fn alloc_zeroed(&self, layout: core::alloc::Layout) -> *mut u8 {}
}
