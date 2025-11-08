use core::{alloc::Layout, num::NonZeroUsize, ops::Deref, ptr::NonNull};

use crate::{
    inline_header::InlineHeader,
    util::{MAX_ALIGN, PAGE_SIZE},
};

/// Represents a memory block
/// The most significant bit of the offset is used to mark whether the block is used
/// Thus you should never access offset field directly, instead, use the provided API
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct UnderlyingLinkedHeader {
    size: usize,
    next: Option<LinkedHeader>,
    offset: usize,
}

impl Default for UnderlyingLinkedHeader {
    fn default() -> Self {
        UnderlyingLinkedHeader::with_size(PAGE_SIZE - size_of::<UnderlyingLinkedHeader>())
    }
}

impl UnderlyingLinkedHeader {
    pub fn with_size(size: usize) -> UnderlyingLinkedHeader {
        UnderlyingLinkedHeader {
            size,
            offset: 0,
            next: None,
        }
    }

    pub fn with_offset(size: usize, offset: usize) -> UnderlyingLinkedHeader {
        UnderlyingLinkedHeader {
            size,
            offset,
            next: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkedHeader(NonNull<UnderlyingLinkedHeader>);

impl InlineHeader for LinkedHeader {
    type Header = UnderlyingLinkedHeader;

    fn new<T: ?Sized>(ptr: *mut T) -> Self {
        if ptr.is_null() {
            panic!("Cannot create ")
        }
        let not_null: NonNull<UnderlyingLinkedHeader> =
            unsafe { NonNull::new_unchecked(ptr).cast() };
        LinkedHeader(not_null)
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

    fn set(&mut self, ptr: LinkedHeader) {
        self.0 = ptr.0
    }

    fn get_data(&self) -> NonNull<u8> {
        let offset = self.get_offset();
        unsafe { self.0.add(1).byte_add(offset).cast::<u8>() }
    }

    fn last_addr(&self) -> usize {
        usize::from(self.addr())
            + size_of::<UnderlyingLinkedHeader>()
            + self.get_offset()
            + self.size()
    }

    unsafe fn next_unchecked(&self) -> Self {
        unsafe { self.read().next.unwrap_unchecked() }
    }

    fn merge_block(&mut self, last_header: &mut Self, required_size: usize, align: usize) -> bool {
        if self.is_last_block() {
            return false;
        }
        let next = unsafe { self.next_unchecked() };
        assert_eq!(next, *last_header);
        let can_merge_blocks =
            unsafe { self.next_unchecked().addr() == NonZeroUsize::new(self.last_addr()).unwrap() };
        if can_merge_blocks {
            let new_size = size_of::<UnderlyingLinkedHeader>()
                + self.get_offset()
                + self.size()
                + last_header.get_offset()
                + last_header.size();
            let fits_with_merge = new_size >= required_size;
            if !fits_with_merge {
                return true;
            }
            let offset = last_header.align_offset(align);
            last_header.set_offset(offset);

            last_header.set_size(new_size);
        }
        true
    }

    fn try_split_allocated_block(&mut self, new_size: usize, last_addr: usize) {
        if !self.can_split_allocated_block(new_size, last_addr) {
            return;
        }

        let offset = self.get_offset();
        let size = self.size();

        let new_header = UnderlyingLinkedHeader::with_size(
            size - offset - new_size - size_of::<UnderlyingLinkedHeader>(),
        );

        unsafe {
            let new_header_ptr = LinkedHeader(self.byte_add(offset + new_size));
            new_header_ptr.write(new_header);

            self.set_next_some(new_header_ptr);
        }

        self.set_size(new_size);
    }

    fn can_split_allocated_block(&self, new_size: usize, last_addr: usize) -> bool {
        let space_for_new_block = self.size() > size_of::<UnderlyingLinkedHeader>() + new_size;
        let within_buffer = self.last_addr() < last_addr;

        space_for_new_block && within_buffer
    }

    fn initialize_header(
        mut page_allocator: impl crate::page_allocator::PageAllocator,
    ) -> *mut Self::Header {
        let base_header = UnderlyingLinkedHeader::with_size(page_allocator.get_page_size());
        unsafe {
            let page: *mut UnderlyingLinkedHeader = page_allocator.request_page_zeroed().cast();
            page.write(base_header);

            page
        }
    }

    fn is_invalid_layout(layout: &Layout) -> bool {
        layout.size() > PAGE_SIZE * 12 && layout.align() > MAX_ALIGN
    }
}

impl From<NonNull<UnderlyingLinkedHeader>> for LinkedHeader {
    fn from(value: NonNull<UnderlyingLinkedHeader>) -> Self {
        LinkedHeader(value)
    }
}

impl LinkedHeader {
    fn is_last_block(&self) -> bool {
        let next = unsafe { self.read().next };
        match next {
            Some(_) => true,
            None => false,
        }
    }

    fn set_next_some(&mut self, next: LinkedHeader) {
        unsafe {
            self.0.as_mut().next = Some(next);
        }
    }

    fn set_next_none(&mut self) {
        unsafe {
            self.0.as_mut().next = None;
        }
    }
}

impl Deref for LinkedHeader {
    type Target = NonNull<UnderlyingLinkedHeader>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{array_page_allocator::ArrayPageAllocator, list_allocator::ListAllocator};
    use core::alloc::{Allocator, Layout};
    use std::boxed::Box;

    #[test]
    fn alloc_chunks() {
        let allocator = ListAllocator::<ArrayPageAllocator, LinkedHeader>::default();
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
        let allocator = ListAllocator::<ArrayPageAllocator, LinkedHeader>::default();
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
        let allocator = ListAllocator::<ArrayPageAllocator, LinkedHeader>::default();
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
        let allocator = ListAllocator::<ArrayPageAllocator, LinkedHeader>::default();
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
        let allocator = ListAllocator::<ArrayPageAllocator, LinkedHeader>::default();
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
        let allocator = ListAllocator::<&mut ArrayPageAllocator, LinkedHeader>::with_allocator(
            &mut page_allocator,
        );
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
        let allocator = ListAllocator::<ArrayPageAllocator, LinkedHeader>::default();
        let mut chunk = Box::<[u8; 16], ListAllocator<ArrayPageAllocator, LinkedHeader>>::new_in(
            [0; 16], allocator,
        );
        chunk[0] = 1;
    }
}
