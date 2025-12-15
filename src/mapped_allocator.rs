use crate::array_page_allocator::ArrayPageAllocator;
use crate::page_allocator::PageAllocator;
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
    const fn last_addr(&self) -> NonNull<u8> {
        unsafe { self.data.add(self.size + self.offset) }
    }
}

/// An allocator in the shape of a mapping between headers and data blocks.
///
/// Holds a buffer of contiguous headers that each
/// point to a not necessarily contiguous memory blocks.
///
/// When the header buffer runs out of space, all the buffers are copied to a new buffer if it
/// can't be expanded in-place.
///
/// When a header is marked as used, if the underlying data block has space remaining, it is split
/// and another header is written to the header buffer to represent the new second block
///
/// When a block is deallocated, the allocator checks for free adjacent blocks and merges any
/// around it into one larger block. This kind of regular, automatic splitting and merging (not just when necessary)
/// fundamentally trades speed for memory efficiency.
///
/// # Usage
/// ## Directly
/// ```rust
/// #![feature(allocator_api)]
/// use yerba::mapped_allocator::MappedAllocator;
/// use core::alloc::{Allocator, Layout};
///
/// let allocator = MappedAllocator::default();
/// let layout = Layout::new::<[u8; 200]>();
/// let chunk = allocator.allocate(layout).unwrap().cast();
/// // Do some stuff
/// unsafe { allocator.deallocate(chunk, layout); }
/// ```
///
/// ## With a smart pointer
/// ```rust
/// #![feature(allocator_api)]
/// use yerba::mapped_allocator::MappedAllocator;
/// use core::alloc::{Allocator, Layout};
///
/// let allocator = MappedAllocator::default();
/// let mut chunk = Box::<[u8; 16], MappedAllocator>::new_in([0; 16], allocator);
/// // Automatically cleaned up on drop
/// ```
pub struct MappedAllocator<'a, A = ArrayPageAllocator<'a>>
where
    A: PageAllocator,
{
    headers: Cell<NonNull<MappedHeader>>,
    header_buffer_size: usize,
    headers_allocated: Cell<usize>,

    page_allocator: RefCell<A>,
    marker: PhantomData<&'a A>,
}

impl<A> MappedAllocator<'_, A>
where
    A: PageAllocator,
{
    const fn headers(&self) -> NonNull<MappedHeader> {
        self.headers.get()
    }

    fn with_allocator(mut page_allocator: A) -> Result<Self, AllocError> {
        let page_size = page_allocator.get_page_size();

        // The pages being allocated are overlapping here
        let blocks_buffer = page_allocator.request_page_zeroed()?.cast::<u8>();

        // NOTE
        // This cast is fine because the given pointer is guaranteed
        // to be aligned to the system's page size
        #[allow(clippy::cast_ptr_alignment)]
        let headers_buffer = page_allocator.request_page_zeroed()?.cast::<MappedHeader>();
        assert!(!headers_buffer.is_null() && !blocks_buffer.is_null());

        let initial_header = MappedHeader {
            data: unsafe { NonNull::new_unchecked(blocks_buffer) },
            size: page_size,
            offset: 0,
            used: false,
        };
        unsafe {
            headers_buffer.write(initial_header);

            Ok(Self {
                headers: Cell::new(NonNull::new_unchecked(headers_buffer)),
                header_buffer_size: page_size,
                headers_allocated: Cell::new(1),
                page_allocator: RefCell::new(page_allocator),
                marker: PhantomData,
            })
        }
    }

    /// Finds an empty block of `size`
    fn find_empty_block(&self, size: usize) -> Option<NonNull<MappedHeader>> {
        let open_and_fits = |header: &MappedHeader| !header.used && header.size >= size;
        self.find_block(open_and_fits)
    }

    fn try_split_block(&self, mut header_ptr: NonNull<MappedHeader>, new_size: usize) {
        let header = unsafe { header_ptr.read() };
        let new_total_size = new_size + header.offset;
        let next_size = header.size - new_total_size;

        // This is the arbitrary place at which we deem it not worth it
        if next_size < size_of::<MappedHeader>() {
            return;
        }

        let new_data_ptr = unsafe { header.data.byte_add(new_total_size) };
        if let Err(err) = self.add_header(new_data_ptr, next_size) {
            println!("Error on adding split header: {err:?}");
            return;
        }

        unsafe {
            header_ptr.as_mut().size = new_total_size;
        }
    }

    fn header_space_remaining(&self) -> bool {
        unsafe {
            self.headers.get().byte_add(self.header_buffer_size)
                > self.headers.get().add(self.headers_allocated.get())
        }
    }

    fn add_header(
        &self,
        data: NonNull<u8>,
        size: usize,
    ) -> Result<NonNull<MappedHeader>, AllocError> {
        let header = MappedHeader {
            size,
            data,
            offset: 0,
            used: false,
        };

        if self.header_space_remaining() {
            unsafe {
                let header_ptr = self.headers().add(self.headers_allocated.get());
                header_ptr.write(header);
                self.headers_allocated.update(|n| n + 1);

                return Ok(header_ptr);
            }
        }

        unsafe {
            // If otherwise, we have to reserve a new buffer, then copy over
            // all of our headers
            //
            // Otherwise, our current header buffer has been expanded and we can safely write
            let extended = self.page_allocator.borrow_mut().extend_page(
                self.headers().as_ptr().cast(),
                size_of::<MappedHeader>() * 12,
            );

            if !extended {
                // NOTE
                // This cast is fine because the given pointer is guaranteed
                // to be aligned to the system's page size
                #[allow(clippy::cast_ptr_alignment)]
                let page = self
                    .page_allocator
                    .borrow_mut()
                    .request_page()?
                    .cast::<MappedHeader>();
                assert!(page.is_null());

                core::ptr::copy_nonoverlapping(
                    self.headers().as_ptr(),
                    page,
                    self.headers_allocated.get(),
                );
                self.headers.set(NonNull::new_unchecked(page));
            }

            // Then, because we know we have enough space, we can just write our header as the last
            // item in the headers buffer
            let header_ptr = self.headers().add(self.headers_allocated.get());
            header_ptr.write(header);
            self.headers_allocated.update(|n| n + 1);

            Ok(header_ptr)
        }
    }

    fn find_block(
        &self,
        predicate: impl Fn(&MappedHeader) -> bool,
    ) -> Option<NonNull<MappedHeader>> {
        let base_ptr = self.headers();
        let mut header;

        for i in 0..self.headers_allocated.get() {
            unsafe {
                let header_ptr = base_ptr.add(i);
                header = header_ptr.read();

                if predicate(&header) {
                    return Some(header_ptr);
                }
            }
        }

        None
    }

    fn find_specific_block(&self, ptr: NonNull<u8>) -> Option<NonNull<MappedHeader>> {
        self.find_block(|header: &MappedHeader| header.data == ptr)
    }

    // TODO
    // For each header, we want to see if it's extendable, if it is we can return.
    // If none are extendable, then we allocate a completely new block
    /// Returns a safe place for a block of size `needed_space` to be in
    /// or an `AllocError`
    fn alloc_more_space(&self, needed_space: usize) -> Result<NonNull<MappedHeader>, AllocError> {
        let extendable = |header: &MappedHeader| unsafe {
            !header.used
                && self
                    .page_allocator
                    .borrow_mut()
                    .extend_page(header.data.as_ptr(), needed_space - header.size)
        };

        let Some(mut header_ptr) = self.find_block(extendable) else {
            let mut allocator = self.page_allocator.borrow_mut();
            let page_size = allocator.get_page_size();

            let page = NonNull::new(allocator.request_page()?).ok_or(AllocError)?;
            return self.add_header(page, page_size);
        };

        unsafe {
            header_ptr.as_mut().size += needed_space;
        }

        Ok(header_ptr)
    }
}

// Where exactly the headers point to in memory isn't really something we care about, so merging
// blocks is especially difficult (but splitting them isn't any harder)
unsafe impl<A> Allocator for MappedAllocator<'_, A>
where
    A: PageAllocator,
{
    fn allocate(&self, layout: core::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();
        let align = layout.align();

        let maybe_block = self.find_empty_block(size);
        // Can't be `unwrap_or_else` because we want to early return
        // if the `alloc_more_space` call fails
        let mut header_ptr = match maybe_block {
            Some(ptr) => ptr,
            None => {
                // TODO Figure out how much space we need exactly (maybe there's some offset that
                // makes this not work
                self.alloc_more_space(size)?
            }
        };

        self.try_split_block(header_ptr, size);
        let header = unsafe { header_ptr.read() };

        let alignment_offset = header.data.align_offset(align);
        let offset_data = unsafe { header.data.add(alignment_offset) };

        let ptr = unsafe { header_ptr.as_mut() };
        ptr.data = offset_data;
        ptr.used = true;

        Ok(NonNull::slice_from_raw_parts(offset_data, size))
    }

    unsafe fn deallocate(&self, ptr: core::ptr::NonNull<u8>, layout: core::alloc::Layout) {
        let Some(mut header_ptr) = self.find_specific_block(ptr) else {
            return;
        };
        unsafe {
            header_ptr.as_mut().used = false;
        }

        // Automatic merging is sort of painful and slow
        unsafe {
            let header = header_ptr.read();

            // TODO Figure out what to do in this case
            assert!(layout.size() == header.size, "Layout of wrong size");
            let get_next_block = |header: &MappedHeader| header.data.add(header.size);

            let is_adjacent_after = |searching_header: &MappedHeader| {
                !searching_header.used && get_next_block(&header) == searching_header.data
            };
            let is_adjacent_before = |searching_header: &MappedHeader| {
                !searching_header.used && get_next_block(searching_header) == header.data
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

impl Default for MappedAllocator<'_> {
    fn default() -> Self {
        let page_allocator = ArrayPageAllocator::default();
        Self::with_allocator(page_allocator).expect("Failed to allocate MappedAllocator")
    }
}

#[cfg(test)]
mod test {
    use core::alloc::Layout;
    use core::hint::black_box;
    use core::{alloc::Allocator, ptr::NonNull};

    use crate::{array_page_allocator::ArrayPageAllocator, mapped_allocator::MappedAllocator};

    #[test]
    fn alloc_chunks() {
        let page_allocator = ArrayPageAllocator::default();
        let allocator = MappedAllocator::with_allocator(page_allocator).unwrap();
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
            let mut one = allocator.allocate(layout).unwrap().cast();
            let two: NonNull<u8> = allocator.allocate(layout).unwrap().cast();

            one = allocator.grow(one, layout, new_layout).unwrap().cast();
            allocator.deallocate(one, new_layout);
            allocator.deallocate(two, layout);
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

            let three = allocator.allocate(layout).unwrap().cast();
            allocator.deallocate(three, layout);
        }
    }

    #[test]
    fn multiple_allocators() {
        let mut page_allocator = ArrayPageAllocator::default();
        let allocator =
            MappedAllocator::<&mut ArrayPageAllocator>::with_allocator(&mut page_allocator)
                .unwrap();
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
        // Somehow, the box is writing into the header buffer
        let chunk = Box::<[u8; 5000], MappedAllocator>::new_in([0; 5000], allocator);
        black_box(chunk);
    }
}
