use crate::mapped_allocator::MappedAllocator;
use core::alloc::Allocator;
use core::ptr::NonNull;

pub struct GCAllocator<'a> {
    allocator: MappedAllocator<'a>,
}

unsafe impl Allocator for GCAllocator<'_> {
    fn allocate(
        &self,
        layout: std::alloc::Layout,
    ) -> Result<std::ptr::NonNull<[u8]>, std::alloc::AllocError> {
        match self.allocator.allocate(layout) {
            Ok(ptr) => Ok(ptr),
            Err(_) => {
                self.collect();
                self.allocate(layout)
            }
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: std::alloc::Layout) {
        todo!()
    }
}

impl GCAllocator<'_> {
    fn collect(&self) {
        let allocator = &self.allocator;
    }
}
