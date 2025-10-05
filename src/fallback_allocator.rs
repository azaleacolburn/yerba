use core::alloc::GlobalAlloc;

/// Holds an allocator of each given type
/// When allocation, reallocation, or deallocation is done, it first calls one allocator, then the
/// other on a failure.
///
/// # Safety
/// - `A` functions must not panic on failure, instead returning a null_ptr or early returning
struct FallbackAllocator<A, F>
where
    A: GlobalAlloc,
    F: GlobalAlloc,
{
    main_allocator: A,
    fallback_allocator: F,
}

unsafe impl<A, F> GlobalAlloc for FallbackAllocator<A, F>
where
    A: GlobalAlloc,
    F: GlobalAlloc,
{
    /// Calls `GlobalAlloc::alloc` on the main allocator, then if that fails, on the fallback allocator
    ///
    /// # Safety
    /// - Neither `A::alloc` nor `F::alloc` may panic if allocation fails, they must instead return a
    /// null pointer
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        unsafe {
            let mut data_ptr = self.main_allocator.alloc(layout);
            if data_ptr.is_null() {
                data_ptr = self.fallback_allocator.alloc(layout);
            }

            data_ptr
        }
    }

    /// Calls `GlobalAlloc::dealloc` on both the main and fallback allocators
    ///
    /// # Safety
    /// - Neither `A::dealloc` nor `F::dealloc` may panic if the specified pointer is not available
    ///     - This may be changed in the future if `FallbackAllocator` is expanded to hold the bounds
    ///     of both sub allocators
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe {
            self.main_allocator.dealloc(ptr, layout);
            self.fallback_allocator.dealloc(ptr, layout);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        unsafe {
            let mut data_ptr = self.main_allocator.realloc(ptr, layout, new_size);
            if data_ptr.is_null() {
                data_ptr = self.fallback_allocator.realloc(ptr, layout, new_size);
            }

            data_ptr
        }
    }

    /// Calls `GlobalAlloc::alloc_zeored` on the main allocator, then if that fails, on the fallback allocator
    ///
    /// # Safety
    /// - Neither `A::alloc_zeored` nor `F::alloc_zeored` may panic if allocation fails, they must instead return a
    /// null pointer
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        unsafe {
            let mut data_ptr = self.main_allocator.alloc_zeroed(layout);
            if data_ptr.is_null() {
                data_ptr = self.fallback_allocator.alloc_zeroed(layout);
            }

            data_ptr
        }
    }
}
