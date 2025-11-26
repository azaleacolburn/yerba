use core::alloc::AllocError;

pub trait WithPageSize
where
    Self: Sized,
{
    fn with_page_size(page_size: usize) -> Result<Self, AllocError>;
}

pub trait WithSize {
    fn with_size(size: usize) -> Self;

    fn with_offset(size: usize, offset: usize) -> Self;
}
