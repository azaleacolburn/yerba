use core::alloc::{AllocError, Allocator};
use core::cell::UnsafeCell;
use core::ptr::NonNull;

pub struct MappedAllocator {
    blocks: *mut UnsafeCell<[u8]>,
    headers: *mut UnsafeCell<[u8]>,
}

unsafe impl Allocator for MappedAllocator {
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        todo!()
    }

    unsafe fn deallocate(&self, ptr: core::ptr::NonNull<u8>, layout: core::alloc::Layout) {
        todo!()
    }
}
