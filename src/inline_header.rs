use crate::page_allocator::PageAllocator;
use core::alloc::Layout;
use core::ops::Deref;
use core::ptr::NonNull;

/// A structure represented a pointer to a header which represents a single block of memory
// NOTE u8 is just a placehoelder, it just needs to be a pointer
pub trait InlineHeader
where
    Self: From<NonNull<Self::Header>> + Deref<Target = NonNull<Self::Header>> + Clone + Copy,
{
    type Header;

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
    fn get_data(&self) -> NonNull<u8>;
    /// Gets the last address in the block represented by this header
    fn last_addr(&self) -> usize;

    /// Gets the next header
    unsafe fn next_unchecked(&self) -> Self;

    /// Merges two consecutive memory blocks in the buffer
    /// self and prev_header must point to contiguous blocks in memory
    fn merge_block(&mut self, prev_header: &mut Self, required_size: usize, align: usize) -> bool;

    /// Attempts to split the allocated block represented
    /// by `header`, into two blocks, the first one of size `new_size`
    ///
    /// Does nothing if there isn't enough space to split the block
    /// Or if `new_size > header.size() + size_of::<Header>()`
    fn try_split_allocated_block(&mut self, new_size: usize, last_addr: usize);
    fn can_split_allocated_block(&self, new_size: usize, last_addr: usize) -> bool;

    /// Allocates a new buffer of size `size` using the given `page_allocator`
    /// then writes a base header to it the buffer
    /// The header will represent the entire allocated buffer
    /// and so will be of size `page_allocator.get_page_size()`
    /// Returns a pointer to that buffer
    fn initialize_header(page_allocator: impl PageAllocator) -> *mut Self::Header;

    /// Returns whether a given layout is valid to by represented by a header of this type
    /// For example, the layout size plus the header size may exceed the maximum layout size specificed by Yerba
    fn is_invalid_layout(layout: &Layout) -> bool;

    fn header_size() -> usize {
        size_of::<Self::Header>()
    }
}
