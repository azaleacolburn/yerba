use core::ptr::NonNull;
use std::{alloc::Layout, ops::Deref};

use crate::{
    inline_header::InlineHeader,
    util::{MAX_ALIGN, MAX_BLOCK_SIZE, MIN_ALIGN, MIN_BLOCK_SIZE, PAGE_SIZE},
};

/// Represents a memory block
/// The most significant bit of the offset is used to mark whether the block is used
/// Thus you should never access offset field directly, instead, use the provided API
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct UnderlyingHeader {
    size: usize,
    offset: usize,
}

impl Default for UnderlyingHeader {
    fn default() -> Self {
        UnderlyingHeader::with_size(PAGE_SIZE - size_of::<UnderlyingHeader>())
    }
}

impl UnderlyingHeader {
    pub fn with_size(size: usize) -> UnderlyingHeader {
        UnderlyingHeader { size, offset: 0 }
    }

    pub fn with_offset(size: usize, offset: usize) -> UnderlyingHeader {
        UnderlyingHeader { size, offset }
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
pub struct ContiguousHeader(NonNull<UnderlyingHeader>);

impl InlineHeader for ContiguousHeader {
    type Header = UnderlyingHeader;

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

    fn get_data(&self) -> NonNull<u8> {
        let offset = self.get_offset();
        unsafe { self.0.add(1).byte_add(offset).cast::<u8>() }
    }

    fn last_addr(&self) -> usize {
        usize::from(self.addr()) + size_of::<UnderlyingHeader>() + self.get_offset() + self.size()
    }

    #[inline]
    unsafe fn next_unchecked(&self) -> ContiguousHeader {
        unsafe {
            self.byte_add(size_of::<UnderlyingHeader>() + self.get_offset() + self.size())
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
        let merged_size = self.size() + last_header.size();
        let fits_with_merge = merged_size >= required_size;

        if fits_with_merge {
            let data_ptr = unsafe { last_header.add(1) };
            let alignment_offset = data_ptr.align_offset(align);
            if alignment_offset == usize::MAX {
                return false;
            }

            last_header.set_offset(alignment_offset);
            last_header.add_size(self.size() + self.get_offset() + size_of::<UnderlyingHeader>());

            unsafe {
                self.write_bytes(0, size_of::<UnderlyingHeader>());
            }
            self.set(*last_header);
        }

        true
    }

    /// Attempts to split the allocated block represented
    /// by `header`, into two blocks, the first one of size `new_size`
    ///
    /// Does nothing if there isn't enough space to split the block
    /// Or if `new_size > header.size() + size_of::<Header>()`
    fn try_split_allocated_block(&mut self, new_size: usize, last_addr: usize) {
        let next_header = unsafe { self.next_unchecked() };
        if !self.can_split_allocated_block(&next_header, new_size, last_addr) {
            return;
        }

        let second_block_size = self.size() - size_of::<UnderlyingHeader>() - new_size;
        self.set_size(new_size);

        let new_header = UnderlyingHeader::with_size(second_block_size);
        unsafe {
            next_header.write(new_header);
        }
    }

    #[inline]
    fn can_split_allocated_block(
        &self,
        next_header: &Self,
        new_size: usize,
        last_addr: usize,
    ) -> bool {
        let space_for_new_block = self.size() > size_of::<UnderlyingHeader>() + new_size;
        let within_buffer =
            (usize::from(next_header.addr()) + size_of::<UnderlyingHeader>()) < last_addr;

        space_for_new_block && within_buffer
    }

    fn initialize_header(
        mut page_allocator: impl crate::page_allocator::PageAllocator,
    ) -> *mut Self::Header {
        const {
            let header_size = size_of::<UnderlyingHeader>();
            assert!(header_size < PAGE_SIZE);
            assert!(header_size % 8 == 0)
        }

        let page_ptr = unsafe {
            page_allocator
                .request_page_zeroed()
                .cast::<UnderlyingHeader>()
        };
        if page_ptr.is_null() {
            panic!("Failed to allocate the first page");
        }

        let head = UnderlyingHeader::with_size(page_allocator.get_page_size());
        unsafe {
            page_ptr.write(head);
        }

        page_ptr
    }

    #[inline]
    fn is_invalid_layout(&layout: &Layout) -> bool {
        let align = layout.align();
        let size = layout.size();
        align > MAX_ALIGN
            || align < MIN_ALIGN
            || size < MIN_BLOCK_SIZE
            || size + size_of::<Self::Header>() > MAX_BLOCK_SIZE
    }
}

impl Deref for ContiguousHeader {
    type Target = NonNull<UnderlyingHeader>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<NonNull<UnderlyingHeader>> for ContiguousHeader {
    fn from(value: NonNull<UnderlyingHeader>) -> Self {
        ContiguousHeader::new(value.as_ptr())
    }
}
