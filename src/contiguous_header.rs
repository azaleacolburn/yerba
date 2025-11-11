use crate::{
    inline_header::InlineHeader,
    util::PAGE_SIZE,
};
use core::ptr::NonNull;
use core::ops::Deref;

/// Represents a memory block
/// The most significant bit of the offset is used to mark whether the block is used
/// Thus you should never access offset field directly, instead, use the provided API
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct UnderlyingContiguousHeader {
    size: usize,
    offset: usize,
}

impl Default for UnderlyingContiguousHeader {
    fn default() -> Self {
        UnderlyingContiguousHeader::with_size(PAGE_SIZE - size_of::<UnderlyingContiguousHeader>())
    }
}

impl UnderlyingContiguousHeader {
    pub fn with_size(size: usize) -> UnderlyingContiguousHeader {
        UnderlyingContiguousHeader { size, offset: 0 }
    }

    pub fn with_offset(size: usize, offset: usize) -> UnderlyingContiguousHeader {
        UnderlyingContiguousHeader { size, offset }
    }
}

// Only allocates a single arena and returns a null pointer for allocations past that
// Allows the arbitrary allocation, deallocation, and reallocation of any block
/// Will merge empty blocks when necessary to fit new allocations
///
/// Used to create an allocator in the shape of a single contiguous buffer, capable of performing allocations, deallocations, and reallocations.
/// It automatically merges and splits free block when suitable.
///
/// While plugging in a custom `PageAllocator` to the parent `ListAllocator` is possible
/// it is not recommended to use one that plans to allocate more than one contiguous page
/// as this allocator could not utilize it fully.
/// This, of course, is not the case if you pass in a specific instance of a `PageAllocator` with
/// `ListALlocator::with_allocator`
///
///# Especially suitable for
/// - Lots of small-midsized allocations/reallocations
///
/// # Limitations
/// - Fails to allocate/reallocate if there isn't enough space in the underlying buffer and the system call to
/// request more contiguous memory fails.
/// - Not suitable for large allocations/reallocations or multithreaded code//
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContiguousHeader(NonNull<UnderlyingContiguousHeader>);

impl InlineHeader for ContiguousHeader {
    type Header = UnderlyingContiguousHeader;

    fn new<T: ?Sized>(ptr: *mut T) -> Self {
        if ptr.is_null() {
            panic!("Tried to create HeaderPtr from null ptr, use HeaderPtr::null() instead")
        }
        let non_null = unsafe { NonNull::new_unchecked(ptr.cast()) };
        Self(non_null)
    }

    fn get_offset(&self) -> usize {
        unsafe { (self.0.read()).offset & (0 as usize) << (size_of::<usize>() * 8 - 1) }
    }

    fn set_offset(&mut self, offset: usize) {
        let used: bool = self.used();
        unsafe {
            self.0.as_mut().offset = offset;
        }
        self.set_used(used);
    }

    fn used(&self) -> bool {
        // Seems to be a bit faster or the same as bitshifting
        unsafe { (self.0.read()).offset.reverse_bits() & 1 == 1 }
    }

    fn set_used(&mut self, used: bool) {
        unsafe {
            let k = size_of::<usize>() * 8 - 1;
            self.0.as_mut().offset &= 0 << k;
            self.0.as_mut().offset &= (used as usize) << k;
        }
    }

    fn size(&self) -> usize {
        unsafe { self.0.read().size }
    }

    fn add_size(&mut self, size: usize) {
        unsafe { self.0.as_mut().size += size }
        // unsafe { (*self.0.write();) += size }
    }

    fn set_size(&mut self, size: usize) {
        unsafe { self.0.as_mut().size = size }
    }

    fn set(&mut self, ptr: ContiguousHeader) {
        self.0 = ptr.0
    }

    #[inline]
    unsafe fn next_unchecked(&self) -> ContiguousHeader {
        unsafe {
            self.byte_add(size_of::<UnderlyingContiguousHeader>() + self.get_offset() + self.size())
                .into()
        }
    }

    /// Merges two consecutive memory blocks in the buffer
    fn merge_block(
        &mut self,
        last_header: &mut ContiguousHeader,
        required_size: usize,
        align: usize,
    ) -> bool {
        let merged_size = self.size() + self.get_offset() + last_header.size();
        let fits_with_merge = merged_size >= required_size;

        if !fits_with_merge {
            return true;
        }

        let data_ptr = unsafe { last_header.add(1) };
        let alignment_offset = data_ptr.align_offset(align);
        if alignment_offset == usize::MAX {
            return false;
        }

        last_header.set_offset(alignment_offset);
        last_header
            .add_size(self.size() + self.get_offset() + size_of::<UnderlyingContiguousHeader>());

        unsafe {
            self.write_bytes(0, size_of::<UnderlyingContiguousHeader>());
        }
        self.set(*last_header);

        true
    }

    /// Attempts to split the allocated block represented
    /// by `header`, into two blocks, the first one of size `new_size`
    ///
    /// Does nothing if there isn't enough space to split the block
    /// Or if `new_size > header.size() + size_of::<Header>()`
    fn try_split_allocated_block(&mut self, new_size: usize, last_addr: usize) {
        let next_header = unsafe { self.next_unchecked() };
        if !self.can_split_allocated_block(new_size, last_addr) {
            return;
        }

        let second_block_size = self.size() - size_of::<UnderlyingContiguousHeader>() - new_size;
        self.set_size(new_size);

        let new_header = UnderlyingContiguousHeader::with_size(second_block_size);
        unsafe {
            next_header.write(new_header);
        }
    }

    #[inline]
    fn can_split_allocated_block(&self, new_size: usize, last_addr: usize) -> bool {
        let space_for_new_block = self.size() > size_of::<UnderlyingContiguousHeader>() + new_size;
        let within_buffer = self.last_addr() < last_addr;

        space_for_new_block && within_buffer
    }

    fn initialize_header(
        mut page_allocator: impl crate::page_allocator::PageAllocator,
    ) -> *mut Self::Header {
        const {
            let header_size = size_of::<UnderlyingContiguousHeader>();
            assert!(header_size < PAGE_SIZE);
            assert!(header_size % 8 == 0)
        }

        let page_ptr = unsafe {
            page_allocator
                .request_page_zeroed()
                .cast::<UnderlyingContiguousHeader>()
        };
        if page_ptr.is_null() {
            panic!("Failed to allocate the first page");
        }

        let head = UnderlyingContiguousHeader::with_size(page_allocator.get_page_size());
        unsafe {
            page_ptr.write(head);
        }

        page_ptr
    }
}

impl Deref for ContiguousHeader {
    type Target = NonNull<UnderlyingContiguousHeader>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<NonNull<UnderlyingContiguousHeader>> for ContiguousHeader {
    fn from(value: NonNull<UnderlyingContiguousHeader>) -> Self {
        ContiguousHeader::new(value.as_ptr())
    }
}

#[cfg(test)]
mod test {
    use crate::{array_page_allocator::ArrayPageAllocator, list_allocator::ListAllocator};

    use super::*;
    use core::alloc::{Allocator, Layout};
    use std::boxed::Box;

    #[test]
    fn alloc_chunks() {
        let allocator = ListAllocator::<ArrayPageAllocator>::default();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let chunk = allocator.allocate(layout).unwrap();
            allocator.deallocate(chunk.cast(), layout);

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
        let allocator = ListAllocator::<ArrayPageAllocator>::default();
        let layout = Layout::new::<[u8; 5000]>();

        unsafe {
            let one = allocator.allocate(layout).unwrap().cast();
            let two = allocator.allocate(layout).unwrap().cast();

            allocator.deallocate(one, layout);
            allocator.deallocate(two, layout);
        }
    }

    #[test]
    fn zeroed() {
        let allocator = ListAllocator::<ArrayPageAllocator>::default();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one: NonNull<u8> = allocator.allocate_zeroed(layout).unwrap().cast();
            let two: NonNull<u8> = allocator.allocate_zeroed(layout).unwrap().cast();

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
        let allocator = ListAllocator::<ArrayPageAllocator>::default();
        let layout = Layout::new::<[u8; 16]>();
        let new_layout = Layout::new::<[u8; 32]>();

        unsafe {
            let one = allocator.allocate(layout).unwrap().cast();
            let two = allocator.allocate(layout).unwrap().cast();

            allocator.grow(two, layout, new_layout);
            allocator.deallocate(one, layout);
            allocator.deallocate(two, new_layout);
        }
    }

    #[test]
    fn merge() {
        let allocator = ListAllocator::<ArrayPageAllocator>::default();
        let layout = Layout::new::<[u8; 2000]>();
        let second_layout = Layout::new::<[u8; 3080]>();

        unsafe {
            let one = allocator.allocate(layout).unwrap().cast();
            allocator.deallocate(one, layout);

            let two = allocator.allocate(second_layout).unwrap().cast();
            allocator.deallocate(two, second_layout);
        }
    }

    #[test]
    fn multiple_allocators() {
        let mut page_allocator = ArrayPageAllocator::default();
        let allocator =
            ListAllocator::<&mut ArrayPageAllocator>::with_allocator(&mut page_allocator);
        let layout = Layout::new::<[u8; 2000]>();
        let second_layout = Layout::new::<[u8; 3080]>();

        unsafe {
            let one = allocator.allocate(layout).unwrap().cast();
            allocator.deallocate(one, layout);

            let two = allocator.allocate(second_layout).unwrap().cast();
            allocator.deallocate(two, second_layout);
        }
    }

    #[test]
    fn with_box() {
        let allocator = ListAllocator::default();
        let mut chunk = Box::<[u8; 16], ListAllocator>::new_in([0; 16], allocator);
        chunk[0] = 1;
    }
}
