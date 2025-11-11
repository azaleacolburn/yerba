use core::alloc::{AllocError, Allocator};
use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ptr::{NonNull, slice_from_raw_parts};

use crate::array_page_allocator::ArrayPageAllocator;
use crate::page_allocator::PageAllocator;

pub struct MappedHeader {
    data: *mut u8,
    size: usize,
    used: bool,
}

pub struct MappedAllocator<'a, A = ArrayPageAllocator<'a>>
where
    A: PageAllocator,
{
    headers: *mut MappedHeader,
    header_buffer_size: usize,
    page_allocator: A,
    marker: PhantomData<&'a A>,
}

impl<'a, A> MappedAllocator<'a, A>
where
    A: PageAllocator,
{
    fn with_allocator(mut allocator: A) -> MappedAllocator<'a, A> {
        let page_size = allocator.get_page_size();
        let blocks_buffer = unsafe { allocator.request_page_zeroed() };
        let headers_buffer = unsafe { allocator.request_page_zeroed().cast::<MappedHeader>() };

        let initial_header = MappedHeader {
            data: blocks_buffer.cast::<u8>(),
            size: page_size,
            used: false,
        };
        unsafe {
            headers_buffer.write(initial_header);
        }

        Self {
            headers: headers_buffer,
            header_buffer_size: page_size,
            page_allocator: allocator,
            marker: PhantomData,
        }
    }

    /// Finds an empty block of `size`
    fn find_empty_block(&self, size: usize) -> Option<*mut MappedHeader> {
        let mut header_ptr = self.headers;
        unsafe {
            let last_addr = self.headers.byte_add(self.header_buffer_size);
            let mut header = header_ptr.read();

            while header.used && header.size < size {
                header_ptr = header_ptr.add(1);

                if header_ptr > last_addr {
                    return None;
                }

                header = header_ptr.read();
            }

            Some(header_ptr)
        }
    }

    fn try_split_block(&mut self, header_ptr: *mut MappedHeader, new_size: usize) {
        let header = unsafe { header_ptr.read() };
        let next_size = header.size - new_size;

        // This is the arbitrary place at which we deep it not worth it
        if next_size < size_of::<MappedHeader>() {
            return;
        }

        // TODO Place a new header at the end of the header_buffer

        unsafe {
            (*header_ptr).size = new_size;
        }
    }
}

unsafe impl<'a, A> Allocator for MappedAllocator<'a, A>
where
    A: PageAllocator,
{
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();
        let align = layout.align();

        let header_ptr = self.find_empty_block(size).ok_or(AllocError)?;

        let header = unsafe { header_ptr.read() };
        self.try_split_block()
    }

    unsafe fn deallocate(&self, ptr: core::ptr::NonNull<u8>, layout: core::alloc::Layout) {
        todo!()
    }
}
