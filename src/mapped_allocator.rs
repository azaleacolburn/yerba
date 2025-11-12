use libc::qsort;

use crate::array_page_allocator::ArrayPageAllocator;
use crate::page_allocator::PageAllocator;
use crate::util::PAGE_SIZE;
use crate::with_page_size::WithPageSize;
use core::alloc::{AllocError, Allocator};
use core::cell::RefCell;
use core::marker::PhantomData;
use core::ptr::NonNull;

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
    headers_allocated: usize,
    page_allocator: RefCell<A>,
    marker: PhantomData<&'a A>,
}

impl<'a, A> MappedAllocator<'a, A>
where
    A: PageAllocator,
{
    fn with_allocator(mut page_allocator: A) -> MappedAllocator<'a, A> {
        let page_size = page_allocator.get_page_size();
        let blocks_buffer = unsafe { page_allocator.request_page_zeroed() };
        let headers_buffer = unsafe { page_allocator.request_page_zeroed().cast::<MappedHeader>() };

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
            headers_allocated: 1,
            page_allocator: RefCell::new(page_allocator),
            marker: PhantomData,
        }
    }

    /// Finds an empty block of `size`
    fn find_empty_block(&self, size: usize) -> Option<*mut MappedHeader> {
        let open_and_fits = |header: MappedHeader| !header.used && header.size >= size;
        self.find_block(open_and_fits)
    }

    fn try_split_block(&self, header_ptr: *mut MappedHeader, new_size: usize) {
        let header = unsafe { header_ptr.read() };
        let next_size = header.size - new_size;

        // This is the arbitrary place at which we deep it not worth it
        if next_size < size_of::<MappedHeader>() {
            return;
        }

        let new_data_ptr = unsafe { header.data.byte_add(new_size) };
        self.add_header(new_data_ptr, next_size);

        unsafe {
            (*header_ptr).size = new_size;
        }
    }

    fn header_space_remaining(&self) -> bool {
        unsafe {
            self.headers.byte_add(self.header_buffer_size)
                > self.headers.add(self.headers_allocated)
        }
    }

    fn add_header(&self, data: *mut u8, size: usize) {
        let header = MappedHeader {
            size,
            data,
            used: false,
        };

        if self.header_space_remaining() {
            unsafe {
                let header_ptr = self.headers.add(self.headers_allocated);
                header_ptr.write(header)
            }

            return;
        }

        unsafe {
            match self
                .page_allocator
                .borrow_mut()
                .extend_page(self.headers.cast(), size_of::<MappedHeader>() * 12)
            {
                // Means we have to copy over our entire old header buffer
                Some(ptr) => core::ptr::copy_nonoverlapping(
                    self.headers,
                    ptr.cast::<MappedHeader>(),
                    self.headers_allocated,
                ),
                // Means our current header buffer has been expanded and we can safely write
                None => self.headers.add(self.headers_allocated).write(header),
            }
        }
    }

    fn find_block(&self, predicate: impl Fn(MappedHeader) -> bool) -> Option<*mut MappedHeader> {
        let mut header_ptr = self.headers;
        unsafe {
            let last_addr = self.headers.byte_add(self.header_buffer_size);
            let mut header = header_ptr.read();

            while !predicate(header) {
                header_ptr = header_ptr.add(1);

                if header_ptr > last_addr {
                    return None;
                }

                header = header_ptr.read();
            }

            Some(header_ptr)
        }
    }

    fn find_specific_block(&self, ptr: *mut u8) -> Option<*mut MappedHeader> {
        self.find_block(|header: MappedHeader| header.data == ptr)
    }

    fn last_addr(header_ptr: *mut MappedHeader) -> *mut u8 {
        unsafe {
            let header = header_ptr.read();
            header.data.add(header.size)
        }
        
    }

    // TODO
    // For each header, we want to see if it's extendable, if it is we can return
    // If none are extendible, then we allocate a completely new block
    fn alloc_more_space(&self, needed_space: usize) -> Result<(), ()> {
        let mut header_ptr = self.headers;
        unsafe {
        let last_header_addr = self.headers.byte_add(self.header_buffer_size);

        let mut allocator = self.page_allocator.borrow_mut();
        while header_ptr < last_header_addr {
            // We're going to make the same call to the page allocator multiple times, which sucks
            let header = header_ptr.read();
                // But We don't want the page to be reallocated here, just extended if possible
            let ptr = match allocator.extend_page(header.data, needed_space) {
                    Some(ptr) => ptr,
                    None => {
                        continue;
                    }
                };

            header_ptr = header_ptr.add(1);
        }
        }

        return Err(())
        
    }
}

// Where exactly the headers point to in memory isn't really something we care about, so merging
// blocks is especially difficult (but splitting them isn't any harder)
unsafe impl<'a, A> Allocator for MappedAllocator<'a, A>
where
    A: PageAllocator,
{
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();
        let align = layout.align();

        let page_size = self.page_allocator.borrow().get_page_size();

        let maybe_block = self.find_empty_block(size);

        let header_ptr = match maybe_block {
            Some(ptr) => ptr,
            None => unsafe {
                                let ptr = match self.page_allocator.borrow_mut().extend_page(, self.page_allocator) {
                    Some(ptr) => ptr,
                    None => self..add(self.headers_allocated)
                };
                if ptr.() {

                }
                self.add_header(data, size);
            }
        };
        .ok_or(AllocError)?;
        self.try_split_block(header_ptr, size);
        let header = unsafe { header_ptr.read() };

        let alignment_offset = header.data.align_offset(align);
        let offset_data = header.data.wrapping_add(alignment_offset);
        unsafe { (*header_ptr).data = offset_data };

        let data_ptr = NonNull::new(offset_data).ok_or(AllocError)?;

        let data_ptr = NonNull::slice_from_raw_parts(data_ptr, size);
        Ok(data_ptr)
    }

    unsafe fn deallocate(&self, ptr: core::ptr::NonNull<u8>, layout: core::alloc::Layout) {
        let header_ptr = self.find_specific_block(ptr.as_ptr()).unwrap();
        unsafe {
            (*header_ptr).used = false;
        }
    }
}

impl<'a> Default for MappedAllocator<'a> {
    fn default() -> Self {
        let page_allocator = ArrayPageAllocator::with_page_size(PAGE_SIZE);
        Self::with_allocator(page_allocator)
    }
}

#[cfg(test)]
mod test {
    use core::alloc::Layout;
    use core::{alloc::Allocator, ptr::NonNull};

    use crate::{
        array_page_allocator::ArrayPageAllocator, mapped_allocator::MappedAllocator,
        util::PAGE_SIZE, with_page_size::WithPageSize,
    };

    #[test]
    fn alloc_chunks() {
        let page_allocator = ArrayPageAllocator::with_page_size(PAGE_SIZE);
        let allocator = MappedAllocator::with_allocator(page_allocator);
        let layout = Layout::new::<[u8; 300]>();

        let one: NonNull<u8> = allocator.allocate(layout).unwrap().cast();
        unsafe {
            one.write_bytes(10, 1000);
            allocator.deallocate(one, layout);
        }
    }

    #[test]
    fn overflow() {
        let allocator = MappedAllocator::default();
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
        let allocator = MappedAllocator::default();
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
        let allocator = MappedAllocator::default();
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
        let allocator = MappedAllocator::default();
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
            MappedAllocator::<&mut ArrayPageAllocator>::with_allocator(&mut page_allocator);
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
        let allocator = MappedAllocator::default();
        let mut chunk = Box::<[u8; 16], MappedAllocator>::new_in([0; 16], allocator);
        chunk[0] = 1;
    }
}
