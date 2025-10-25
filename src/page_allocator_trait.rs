pub trait PageAllocator {
    fn new(page: usize) -> Self;
    unsafe fn request_page(&mut self) -> *mut u8;
    unsafe fn request_page_zeroed(&mut self) -> *mut u8;
    unsafe fn relinquish_page(&mut self, ptr: *mut u8);
}
