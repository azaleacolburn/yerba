use core::alloc::AllocError;

pub trait PageAllocator {
    unsafe fn request_page(&mut self) -> Result<*mut u8, AllocError>;
    unsafe fn request_page_zeroed(&mut self) -> Result<*mut u8, AllocError>;
    unsafe fn relinquish_page(&mut self, ptr: *mut u8);
    /// Attempts to extend a given allocated block by `added_size`, returns true if it was able to
    /// extend the block, false otherwise
    /// Does not attempt to allocate a new page if the current one could not be extended
    unsafe fn extend_page(&mut self, ptr: *mut u8, added_size: usize) -> bool;
    fn get_pages_allocated(&self) -> usize;
    fn get_page_size(&self) -> usize;

    /// Creates a "by reference" adapter for this instance of `PageAllocator`.
    ///
    /// The returned adapter also implements `PageAllocator` and will simply borrow this.
    fn by_ref(&self) -> &Self
    where
        Self: Sized,
    {
        self
    }
}

impl<A> PageAllocator for &mut A
where
    A: PageAllocator,
{
    #[inline]
    unsafe fn request_page(&mut self) -> Result<*mut u8, AllocError> {
        unsafe { (**self).request_page() }
    }

    #[inline]
    unsafe fn request_page_zeroed(&mut self) -> Result<*mut u8, AllocError> {
        unsafe { (**self).request_page_zeroed() }
    }

    #[inline]
    unsafe fn relinquish_page(&mut self, ptr: *mut u8) {
        unsafe { (**self).relinquish_page(ptr) }
    }

    #[inline]
    unsafe fn extend_page(&mut self, ptr: *mut u8, added_size: usize) -> bool {
        unsafe { (**self).extend_page(ptr, added_size) }
    }

    #[inline]
    fn get_pages_allocated(&self) -> usize {
        (**self).get_pages_allocated()
    }

    #[inline]
    fn get_page_size(&self) -> usize {
        (**self).get_page_size()
    }
}
