use core::alloc::{AllocError, Allocator, Layout};
use core::ptr::NonNull;

/// Holds an allocator of each given type
/// When allocation, reallocation, or deallocation is done, it first calls one allocator, then the
/// other on a failure.
///
/// # Safety
/// - `A` functions must not panic on failure, instead must return an `AllocError` or early return
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
    ///   null pointer
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
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
    ///       of both sub allocators
    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        unsafe {
            self.main_allocator.deallocate(ptr, layout);
            self.fallback_allocator.deallocate(ptr, layout);
        }
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        layout: Layout,
        new_layout: Layout,
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

    use crate::{list_allocator::ListAllocator, stack_allocator::StackAllocator};

    use super::*;

    #[test]
    fn alloc_chunks() {
        let allocator = FallbackAllocator::<StackAllocator, ListAllocator>::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let chunk = allocator.allocate(layout).unwrap().cast();
            allocator.deallocate(chunk, layout);

            let one = allocator.allocate(layout).unwrap().cast();
            let two = allocator.allocate(layout).unwrap().cast();
            let three = allocator.allocate(layout).unwrap().cast();

            allocator.deallocate(three, layout);
            allocator.deallocate(one, layout);
            allocator.deallocate(two, layout);
        }
    }

    #[test]
    fn overflow() {
        let allocator = FallbackAllocator::<StackAllocator, ListAllocator>::new();
        let layout = Layout::new::<[u8; 5000]>();

        unsafe {
            let one = allocator.allocate(layout).unwrap().cast();
            allocator.deallocate(one, layout);

            let two = allocator.allocate(layout).unwrap().cast();
            allocator.deallocate(two, layout);
        }
    }

    #[test]
    fn zeroed() {
        let allocator = FallbackAllocator::<StackAllocator, ListAllocator>::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.allocate_zeroed(layout).unwrap().cast();
            let two = allocator.allocate_zeroed(layout).unwrap().cast();

            let two_sum: u8 = (0..16).into_iter().map(|i| two.add(i).read()).sum();
            let one_sum: u8 = (0..16).into_iter().map(|i| one.add(i).read()).sum();
            assert_eq!(two_sum, 0);
            assert_eq!(one_sum, 0);

            allocator.deallocate(two, layout);
            allocator.deallocate(one, layout);
        }
    }

    #[test]
    fn realloc() {
        let allocator = FallbackAllocator::<StackAllocator, ListAllocator>::new();
        let layout = Layout::new::<[u8; 16]>();
        let second_layout = Layout::new::<[u8; 32]>();

        unsafe {
            let one = allocator.allocate(layout).unwrap().cast();
            let two = allocator.allocate(layout).unwrap().cast();

            allocator.grow(two, layout, second_layout);
            allocator.deallocate(one, layout);
            allocator.deallocate(two, second_layout);
        }
    }

    #[test]
    fn merge() {
        let allocator = FallbackAllocator::<StackAllocator, ListAllocator>::new();
        let layout = Layout::new::<[u8; 2000]>();
        let second_layout = Layout::new::<[u8; 3080]>();

        unsafe {
            let one = allocator.allocate(layout).unwrap().cast();
            allocator.deallocate(one, layout);

            let two = allocator.allocate(second_layout).unwrap().cast();
            allocator.deallocate(two, layout);
        }
    }
}
