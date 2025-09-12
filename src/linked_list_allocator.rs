use core::alloc::GlobalAlloc;

/// Represents a memory block
struct Block {
    size: usize,
    used: bool,
    block: *const Block,
    data: *mut u8,
}

struct LinkedListAllocator {
    head: *mut Block,
}

unsafe impl GlobalAlloc for LinkedListAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {}

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
