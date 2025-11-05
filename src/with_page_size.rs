pub trait WithPageSize {
    fn with_page_size(page_size: usize) -> Self;
}

pub trait WithSize {
    fn with_size(size: usize) -> Self;

    fn with_offset(size: usize, offset: usize) -> Self;
}
