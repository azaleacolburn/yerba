pub trait PageAllocator: Send + Sync {
    fn new(page: usize) -> Self;
    unsafe fn request_page(&mut self) -> *mut u8;
    unsafe fn request_page_zeroed(&mut self) -> *mut u8;
    unsafe fn relinquish_page(&mut self, ptr: *mut u8);
    fn get_pages_allocated(&self) -> usize;
    fn get_page_size(&self) -> usize;
}
