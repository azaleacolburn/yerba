use crate::array_page_allocator::ArrayPageAllocator;
use crate::page_allocator::PageAllocator;
use crate::util::PAGE_SIZE;
use crate::with_page_size::WithPageSize;
use core::alloc::{AllocError, Allocator};
use core::cell::{Cell, RefCell};
use core::marker::PhantomData;
use core::ptr::NonNull;

#[derive(Debug, Clone)]
pub struct MappedHeader {
    data: NonNull<u8>,
    size: usize,
    offset: usize,
    used: bool,
}

impl MappedHeader {
    fn last_addr(&self) -> NonNull<u8> {
        unsafe { self.data.add(self.size + self.offset) }
    }
}

pub struct MappedAllocator<'a, A = ArrayPageAllocator<'a>>
where
    A: PageAllocator,
{
    headers: Cell<NonNull<MappedHeader>>,
    header_buffer_size: usize,
    headers_allocated: usize,

    page_allocator: RefCell<A>,
    marker: PhantomData<&'a A>,
}

impl<'a, A> MappedAllocator<'a, A>
where
    A: PageAllocator,
{
    fn headers(&self) -> NonNull<MappedHeader> {
        self.headers.get()
    }

    fn with_allocator(mut page_allocator: A) -> MappedAllocator<'a, A> {
        let page_size = page_allocator.get_page_size();

        let blocks_buffer = unsafe { page_allocator.request_page_zeroed().cast::<u8>() };
        let headers_buffer = unsafe { page_allocator.request_page_zeroed().cast::<MappedHeader>() };
        assert!(!headers_buffer.is_null() && !blocks_buffer.is_null());

        let initial_header = MappedHeader {
            data: unsafe { NonNull::new_unchecked(blocks_buffer) },
            size: page_size,
            offset: 0,
            used: false,
        };
        unsafe {
            headers_buffer.write(initial_header);

            Self {
                headers: Cell::new(NonNull::new_unchecked(headers_buffer)),
                header_buffer_size: page_size,
                headers_allocated: 1,
                page_allocator: RefCell::new(page_allocator),
                marker: PhantomData,
            }
        }
    }

    /// Finds an empty block of `size`
    fn find_empty_block(&self, size: usize) -> Option<NonNull<MappedHeader>> {
        let open_and_fits = |header: MappedHeader| !header.used && header.size >= size;
        self.find_block(open_and_fits)
    }

    fn try_split_block(&self, mut header_ptr: NonNull<MappedHeader>, new_size: usize) {
        let header = unsafe { header_ptr.read() };
        let next_size = header.size - new_size;

        // This is the arbitrary place at which we deep it not worth it
        if next_size < size_of::<MappedHeader>() {
            return;
        }

        let new_data_ptr = unsafe { header.data.byte_add(new_size) };
        self.add_header(new_data_ptr, next_size);

        unsafe {
            header_ptr.as_mut().size = new_size;
        }
    }

    fn header_space_remaining(&self) -> bool {
        unsafe {
            self.headers.get().byte_add(self.header_buffer_size)
                > self.headers.get().add(self.headers_allocated)
        }
    }

    fn add_header(&self, data: NonNull<u8>, size: usize) -> NonNull<MappedHeader> {
        let header = MappedHeader {
            size,
            data,
            offset: 0,
            used: false,
        };

        if self.header_space_remaining() {
            unsafe {
                let header_ptr = self.headers.get().add(self.headers_allocated);
                header_ptr.write(header);

                return header_ptr;
            }
        }

        unsafe {
            // If true, we have to reserve a new buffer then copy over
            // all our headers
            //
            // Otherwise, our current header buffer has been expanded and we can safely write
            let extended = self.page_allocator.borrow_mut().extend_page(
                self.headers().as_ptr().cast(),
                size_of::<MappedHeader>() * 12,
            );

            if !extended {
                let page = self
                    .page_allocator
                    .borrow_mut()
                    .request_page()
                    .cast::<MappedHeader>();
                assert!(page.is_null());

                core::ptr::copy_nonoverlapping(
                    self.headers().as_ptr(),
                    page,
                    self.headers_allocated,
                );
                self.headers.set(NonNull::new_unchecked(page));
            }

            // Then, because we know we have enough space, we can just write our header as the last
            // item in the headers buffer
            let header_ptr = self.headers().add(self.headers_allocated);
            header_ptr.write(header);

            header_ptr
        }
    }

    fn find_block(
        &self,
        predicate: impl Fn(MappedHeader) -> bool,
    ) -> Option<NonNull<MappedHeader>> {
        let mut header_ptr = self.headers();

        unsafe {
            let last_addr = self.headers().byte_add(self.header_buffer_size);
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

    fn find_specific_block(&self, ptr: NonNull<u8>) -> Option<NonNull<MappedHeader>> {
        self.find_block(|header: MappedHeader| header.data == ptr)
    }

    // TODO
    // For each header, we want to see if it's extendable, if it is we can return
    // If none are extendible, then we allocate a completely new block
    /// Returns a safe place for a block of size `needed_space` to be alloced in
    /// or an `AllocError`
    fn alloc_more_space(&self, needed_space: usize) -> Result<NonNull<u8>, AllocError> {
        let mut header_ptr = self.headers.get();
        unsafe {
            let last_header_addr = self.headers().byte_add(self.header_buffer_size);

            let mut allocator = self.page_allocator.borrow_mut();
            while header_ptr < last_header_addr {
                // We're going to make the same call to the page allocator multiple times, which sucks
                let header = header_ptr.read();
                let extended = allocator.extend_page(header.data.as_ptr(), needed_space);
                // If the page has been extended, then we can just write to the last address after
                // the current header_ptr
                if extended {
                    return Ok(header.last_addr());
                }

                header_ptr = header_ptr.add(1);
            }
        }

        return Err(AllocError);
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

        let maybe_block = self.find_empty_block(size);
        let mut header_ptr = match maybe_block {
            Some(ptr) => ptr,
            None => {
                // TODO Figure out how much space we need exactly (maybe there's some offset that
                // makes this not work
                let data_ptr = self.alloc_more_space(size)?;
                self.add_header(data_ptr, size)
            }
        };

        self.try_split_block(header_ptr, size);
        let header = unsafe { header_ptr.read() };

        let alignment_offset = header.data.align_offset(align);
        let offset_data = unsafe { header.data.add(alignment_offset) };
        unsafe { header_ptr.as_mut().data = offset_data };

        let data_ptr = NonNull::slice_from_raw_parts(offset_data, size);

        Ok(data_ptr)
    }

    unsafe fn deallocate(&self, ptr: core::ptr::NonNull<u8>, _layout: core::alloc::Layout) {
        let mut header_ptr = self.find_specific_block(ptr).unwrap();
        unsafe {
            header_ptr.as_mut().used = false;
        }

        // TODO
        // Writing automatic merging is going to be sort of painful and slow

        unsafe {
            let header = header_ptr.read();
            let get_next = |header: &MappedHeader| header.data.add(header.size);

            let is_adjacent_after = |searching_header: MappedHeader| {
                !searching_header.used && get_next(&header) == searching_header.data
            };
            let is_adjacent_before = |searching_header: MappedHeader| {
                !searching_header.used && get_next(&searching_header) == header.data
            };

            // This traverses twice, which is annoying
            let adjacent_after = self.find_block(is_adjacent_after);
            if let Some(next_header_ptr) = adjacent_after {
                // Absorb the next header
                let next_header = next_header_ptr.read();
                header_ptr.as_mut().size += next_header.size;

                // Clear the next header
                next_header.data.write_bytes(0, next_header.size);
                next_header_ptr.write_bytes(0, size_of::<MappedHeader>());
                // TODO Find some way to filter out empty blocks unless we want to shift them all
                // down or smth ewww
            }

            let adjacent_before = self.find_block(is_adjacent_before);
            if let Some(mut prev_header_ptr) = adjacent_before {
                let header = header_ptr.read();
                prev_header_ptr.as_mut().size += header.size;

                header_ptr.write_bytes(0, size_of::<MappedHeader>());
            }
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
