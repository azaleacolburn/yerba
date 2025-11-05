use core::ops::Deref;
use core::ptr::NonNull;

/// A structure represented a pointer to a header which represents a single block of memory
// NOTE u8 is just a placehoelder, it just needs to be a pointer
pub trait InlineHeader<Header>
where
    Self: From<NonNull<Header>> + Deref<Target = NonNull<Header>> + Clone + Copy,
    Header: Clone + Copy,
{
    fn new<T: ?Sized>(ptr: *mut T) -> Self;

    /// Gets the offset of the memory in the block represented by this header
    fn get_offset(&self) -> usize;
    /// Sets the offset of the memory in the block represented by this header
    fn set_offset(&mut self, offset: usize);

    /// Returns whether the block is currently used or free for use
    fn used(&self) -> bool;
    /// Sets whether the block is currently used or free for use
    fn set_used(&mut self, used: bool);

    /// Sets the block as free
    fn mark_free(&mut self) {
        self.set_used(false);
    }
    /// Sets the block as used
    fn mark_used(&mut self) {
        self.set_used(true);
    }

    /// Gets the size of the block, not including the offset
    fn size(&self) -> usize;
    /// Sets the size of the block, not including the offset
    fn set_size(&mut self, size: usize);
    /// Increments the size of the block, not including the offset
    fn add_size(&mut self, size: usize);

    fn set(&mut self, size: Self);

    // /// Sets the value of the pointer
    // pub fn set(&mut self, ptr: &HeaderPtr);

    /// Gets the underlying block represented by the header
    fn get_data(&self) -> *mut u8;
    /// Gets the last address in the block represented by this header
    fn last_addr(&self) -> usize;

    /// Gets the next header
    unsafe fn next_unchecked(&self) -> Self;

    /// Merges two consecutive memory blocks in the buffer
    fn merge_block(&mut self, last_header: &mut Self, required_size: usize, align: usize) -> bool;
}

// impl Deref for T
// where
//     T: InlineHeader,
// {
//     type Target = NonNull<Header>;
//     fn deref(&self) -> &Self::Target {
//         &self.0
//     }
// }
//
// impl From<NonNull<Header>> for HeaderPtr {
//     fn from(value: NonNull<Header>) -> Self {
//         HeaderPtr(value)
//     }
// }
