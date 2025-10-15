pub trait PageAllocator {
    fn new(page: usize) -> Self;
    unsafe fn request_page(&self) -> *mut u8;
    unsafe fn request_page_zeroed(&self) -> *mut u8;
    unsafe fn relinquish_page(&self, ptr: *mut u8);
}
