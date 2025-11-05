use core::ptr::NonNull;
use std::{marker::PhantomData, ops::Deref};

use crate::{inline_header::InlineHeader, util::PAGE_SIZE};

/// Represents a memory block
/// The most significant bit of the offset is used to mark whether the block is used
/// Thus you should never access offset field directly, instead, use the provided API
#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Header {
    size: usize,
    offset: usize,
}

impl Default for Header {
    fn default() -> Self {
        Header::new(PAGE_SIZE - size_of::<Header>())
    }
}

impl Header {
    pub fn new(size: usize) -> Header {
        Header { size, offset: 0 }
    }

    pub fn with_offset(size: usize, offset: usize) -> Header {
        Header { size, offset }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContiguousHeader<G>(NonNull<Header>, PhantomData<G>);

impl InlineHeader<Header> for ContiguousHeader<Header> {
    fn new<T: ?Sized>(ptr: *mut T) -> Self {
        if ptr.is_null() {
            panic!("Tried to create HeaderPtr from null ptr, use HeaderPtr::null() instead")
        }
        let non_null = unsafe { NonNull::new_unchecked(ptr.cast()) };
        Self(non_null, PhantomData)
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

    fn get_data(&self) -> *mut u8 {
        let offset = self.get_offset();
        unsafe { self.add(1).byte_add(offset).cast::<u8>().as_ptr() }
    }

    fn last_addr(&self) -> usize {
        usize::from(self.addr()) + size_of::<Header>() + self.get_offset() + self.size()
    }

    unsafe fn next_unchecked(&self) -> ContiguousHeader<Header> {
        unsafe {
            self.byte_add(size_of::<Header>() + self.get_offset() + self.size())
                .into()
        }
    }

    /// Merges two consecutive memory blocks in the buffer
    fn merge_block(
        &mut self,
        last_header: &mut ContiguousHeader<Header>,
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
            last_header.add_size(self.size() + self.get_offset() + size_of::<Header>());

            unsafe {
                self.write_bytes(0, size_of::<Header>());
            }
            self.set(*last_header);
        }

        true
    }
}

impl Deref for ContiguousHeader<Header> {
    type Target = NonNull<Header>;
    fn deref(&self) -> &Self::Target {
        unsafe { self.0.cast().read() }
    }
}

impl From<NonNull<Header>> for ContiguousHeader<Header> {
    fn from(value: NonNull<Header>) -> Self {
        ContiguousHeader::new(value.as_ptr())
    }
}
