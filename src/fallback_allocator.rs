use core::alloc::Allocator;
use core::alloc::GlobalAlloc;
use std::alloc::AllocError;
use std::ptr::NonNull;

/// Holds an allocator of each given type
/// When allocation, reallocation, or deallocation is done, it first calls one allocator, then the
/// other on a failure.
///
/// # Safety
/// - `A` functions must not panic on failure, instead returning a null_ptr or early returning
struct FallbackAllocator<A, F>
where
    A: Allocator,
    F: Allocator,
{
    main_allocator: A,
    fallback_allocator: F,
}

impl<A, F> FallbackAllocator<A, F>
where
    A: Allocator + Default,
    F: Allocator + Default,
{
    fn new() -> Self {
        Self {
            main_allocator: A::default(),
            fallback_allocator: F::default(),
        }
    }
}

unsafe impl<A, F> Allocator for FallbackAllocator<A, F>
where
    A: Allocator,
    F: Allocator,
{
    /// Calls `GlobalAlloc::alloc` on the main allocator, then if that fails, on the fallback allocator
    ///
    /// # Safety
    /// - Neither `A::alloc` nor `F::alloc` may panic if allocation fails, they must instead return a
    /// null pointer
    fn allocate(&self, layout: std::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        let data_ptr = self.main_allocator.allocate(layout);
        match data_ptr {
            Ok(ptr) => Ok(ptr),
            Err(_) => Ok(self.fallback_allocator.allocate(layout)?),
        }
    }

    /// Calls `GlobalAlloc::dealloc` on both the main and fallback allocators
    ///
    /// # Safety
    /// - Neither `A::dealloc` nor `F::dealloc` may panic if the specified pointer is not available
    ///     - This may be changed in the future if `FallbackAllocator` is expanded to hold the bounds
    ///     of both sub allocators
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: std::alloc::Layout) {
        unsafe {
            self.main_allocator.deallocate(ptr, layout);
            self.fallback_allocator.deallocate(ptr, layout);
        }
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        layout: std::alloc::Layout,
        new_layout: std::alloc::Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        unsafe {
            let data_ptr = self.main_allocator.grow(ptr, layout, new_layout);
            match data_ptr {
                Ok(ptr) => Ok(ptr),
                Err(_) => Ok(self.fallback_allocator.grow(ptr, layout, new_layout)?),
            }
        }
    }
}

#[cfg(test)]
mod test {
    use core::alloc::Layout;

    use crate::{linked_list_allocator::ContiguousListAllocator, stack_allocator::StackAllocator};

    use super::*;

    #[test]
    fn alloc_chunks() {
        let allocator = FallbackAllocator::<StackAllocator, ContiguousListAllocator>::new();
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
            allocator.dealloc(one, layout);
            allocator.dealloc(two, layout);
        }
    }

    #[test]
    fn overflow() {
        let allocator = FallbackAllocator::<StackAllocator, ContiguousListAllocator>::new();
        let layout = Layout::new::<[u8; 5000]>();

        unsafe {
            let one = allocator.alloc(layout);
            assert!(!one.is_null());
            allocator.dealloc(one, layout);

            let two = allocator.alloc(layout);
            assert!(!two.is_null());
            allocator.dealloc(two, layout);
        }
    }

    #[test]
    fn zeroed() {
        let allocator = FallbackAllocator::<StackAllocator, ContiguousListAllocator>::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc_zeroed(layout);
            assert!(!one.is_null());

            let two = allocator.alloc_zeroed(layout);
            assert!(!two.is_null());

            let two_sum: u8 = (0..16).into_iter().map(|i| *(two.wrapping_add(i))).sum();
            let one_sum: u8 = (0..16).into_iter().map(|i| *(one.wrapping_add(i))).sum();
            assert_eq!(two_sum, 0);
            assert_eq!(one_sum, 0);

            allocator.dealloc(two, layout);
            allocator.dealloc(one, layout);
        }
    }

    #[test]
    fn realloc() {
        let allocator = FallbackAllocator::<StackAllocator, ContiguousListAllocator>::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc(layout);
            assert!(!one.is_null());

            let two = allocator.alloc(layout);
            assert!(!two.is_null());

            allocator.realloc(two, layout, 32);
            allocator.dealloc(one, layout);
            allocator.dealloc(two, Layout::new::<[u8; 32]>());
        }
    }

    #[test]
    fn merge() {
        let allocator = FallbackAllocator::<StackAllocator, ContiguousListAllocator>::new();
        let layout = Layout::new::<[u8; 2000]>();

        unsafe {
            let one = allocator.alloc(layout);
            assert!(!one.is_null());
            allocator.dealloc(one, layout);

            let layout = Layout::new::<[u8; 3080]>();
            let two = allocator.alloc(layout);
            assert!(!two.is_null());
        }
    }
}
