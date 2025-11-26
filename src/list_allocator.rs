use crate::array_page_allocator::ArrayPageAllocator;
use crate::contiguous_header::ContiguousHeader;
use crate::inline_header::InlineHeader;
use crate::page_allocator::PageAllocator;
use crate::util::PAGE_SIZE;
use crate::with_page_size::WithPageSize;
use core::alloc::{AllocError, Allocator, Layout};
use core::cell::RefCell;
use core::marker::PhantomData;
use core::ptr::NonNull;
use core::{cell::UnsafeCell, ptr::slice_from_raw_parts_mut};

// NOTE Explicitly dropping is not important because
// all the underlying memory is deallocated by the page allocator

/// A general allocator where headers to memory blocks are inlined to the buffer, rather than being
/// stored in an external map.
/// Functionality can depend widely on which `InlineHeader` is used.
///
/// The generic header struct controls:
/// - How the next header is accessed
/// - How new memory blocks are allocated
/// - How/if blocks can be merged and split
///
/// # Use Cases
/// - This is a versatile, single-threaded allocator with few limitations
/// - The exact suitability depends on the `InlineHeader`
///
/// ## Limitations
/// - Depends on the `InlineHeader` chosen
///
/// # Usage
/// ## Direct Allocation
/// ```rust
/// #![feature(allocator_api)]
/// use yerba::{list_allocator::ListAllocator, array_page_allocator::ArrayPageAllocator};
/// use core::alloc::{Allocator, Layout};
///
/// let allocator = ListAllocator::<ArrayPageAllocator>::default();
/// let layout = Layout::new::<[u8; 16]>();
///
/// unsafe {
///     let chunk = allocator.allocate_zeroed(layout).unwrap().cast();
///     chunk.write_bytes(1, 16);
///     allocator.deallocate(chunk, layout);
/// }
/// ```
///
/// ## Use in a structure
/// ```rust
/// #![feature(allocator_api)]
/// use yerba::list_allocator::ListAllocator;
///
/// let allocator = ListAllocator::default();
/// let mut chunk = Box::<[u8; 16], ListAllocator>::new_in([0; 16], allocator);
/// chunk[0] = 1;
/// ```
///
/// The default header is the `ContiguousHeader`, which organizes the `ListAllocator` as a single
/// contiguous list with inlined headers.
///
/// Other options include
pub struct ListAllocator<'a, A = ArrayPageAllocator<'a>, H = ContiguousHeader>
where
    A: PageAllocator,
    H: InlineHeader,
{
    buf: *mut UnsafeCell<[u8]>,
    page_allocator: RefCell<A>,
    phantom: PhantomData<&'a H>,
}

impl<'a, A: PageAllocator, H: InlineHeader> ListAllocator<'a, A, H> {
    /// Creates a new contiguous list allocator with a given `PageAllocator` instance
    ///
    /// # Safety
    /// Panics if:
    /// - The first page cannot be allocated
    pub fn with_allocator(mut page_allocator: A) -> Self {
        let first_block = H::initialize_header(&mut page_allocator);

        let buf = slice_from_raw_parts_mut(first_block, PAGE_SIZE) as *mut UnsafeCell<[u8]>;

        Self {
            buf,
            page_allocator: RefCell::new(page_allocator),
            phantom: PhantomData,
        }
    }

    fn next_header(&self, header_ptr: &H) -> Option<H> {
        if header_ptr.size() == 0 {
            panic!("Should not have zero sized headers")
        }
        if header_ptr.last_addr() >= self.last_addr() {
            return None;
        }
        Some(unsafe { header_ptr.next_unchecked() })
    }

    /// Requests a new page to accommodate a new block started at the old last_addr
    /// Places both a new header representing a block of `size` and `alignment_offset`
    /// Then creates a new top header with the remaining size
    /// # Args
    /// - `header_ptr`: the header pointer to be ultimately returned
    /// - `size`: the requested size of the block the header pointer will represent
    /// - `alignment_offset`: the calculated offset to be added to
    ///     the `data_ptr` that `header_ptr` represents, to align it to `T`, where `data_ptr`
    ///     is of type `*mut T`
    /// # Safety
    /// Panics if:
    /// - a new page contiguous page cannot be allocated
    pub fn add_page(&self, size: usize) -> Result<(), AllocError> {
        unsafe {
            let mut header = self.last_block();
            let initial_header_size = header.size();
            let old_last_addr = self.last_addr();

            let new_page = self.page_allocator.borrow_mut().request_page();
            // Fails if the new page is null or not contiguous with the old one
            if new_page.is_null() {
                return Err(AllocError);
            }

            header.set_size(initial_header_size + PAGE_SIZE);
            header.try_split_allocated_block(size, self.last_addr());

            let allocated_space = self.last_addr() - self.buf_ptr().addr();
            assert_eq!(2 * PAGE_SIZE, allocated_space);

            // NOTE This should be covered by spliting the expanded top block
            // let remaining_size = self.last_addr()
            //     - size_of::<Header>() * 2
            //     - alignment_offset
            //     - size
            //     - self.buf_ptr().addr();
            //
            // let new_top_header = Header::new(remaining_size);
            // let top_header_ptr = self.next_header(&old_last_header);
            // assert!(!top_header_ptr.is_null());
            // top_header_ptr.write(new_top_header);
        }

        Ok(())
    }

    fn last_block(&self) -> H {
        let mut frontier = self.first_block();
        let mut next = self.next_header(&frontier);
        while let Some(next_ptr) = next {
            frontier = next_ptr;
            next = self.next_header(&frontier);
        }

        frontier
    }

    /// Gets the next block in the array, even if it's not initialized
    ///
    /// # Returns
    /// - The first empty header pointer that accomodates `size` in the allocator.
    /// - A null pointer if unable to create an offset that aligns data pointer to `align`
    ///
    fn find_empty_block(&self, size: usize, align: usize) -> Result<H, AllocError> {
        let mut last_header_ptr: Option<H> = None;
        let mut curr_header_ptr: Option<H> = Some(self.first_block());

        while let Some(ref mut header_ptr) = curr_header_ptr {
            if header_ptr.used() {
                last_header_ptr.replace(*header_ptr);
                // If the block is used, there must be another block
                let next_block = &self.next_header(&header_ptr).unwrap();
                curr_header_ptr.replace(*next_block);

                continue;
            }

            // We don't actually use this pointer again, it's just for calculating the offset
            let data_ptr = unsafe { header_ptr.add(1).cast::<u8>() };
            let alignment_offset = data_ptr.align_offset(align);
            if alignment_offset == usize::MAX {
                return Err(AllocError);
            }

            let required_size = size + alignment_offset;
            let fits = header_ptr.size() >= required_size;

            // We've found a block that fits
            if fits {
                header_ptr.set_offset(alignment_offset);

                break;
            }

            // We've found a pair of free blocks that can be merged to fit
            if let Some(mut last_header_ptr) = last_header_ptr
                && !last_header_ptr.used()
            {
                let merge_failed =
                    !header_ptr.merge_block(&mut last_header_ptr, required_size, align);
                if merge_failed {
                    return Err(AllocError);
                }

                break;
            }

            last_header_ptr = Some(*header_ptr);
            let next_header = &self.next_header(&header_ptr);
            if let None = next_header {
                let pre = self.last_addr();
                self.add_page(size)?;
                let post = self.last_addr();
                assert_eq!(post - pre, PAGE_SIZE);

                break;
            }
            curr_header_ptr = *next_header;
        }

        curr_header_ptr.ok_or_else(|| AllocError)
    }

    #[inline]
    fn first_block(&self) -> H {
        H::new(self.buf_ptr())
    }

    #[inline]
    fn last_addr(&self) -> usize {
        let pages = self.page_allocator.borrow().get_pages_allocated();
        self.buf_ptr().wrapping_add(PAGE_SIZE * pages).addr()
    }

    #[inline]
    fn buf_ptr(&self) -> *mut u8 {
        unsafe { (*self.buf).get().cast() }
    }

    /// Finds the block representing the given data pointer
    /// If it does not exist, null is returned instead
    fn find_ptr_block(&self, ptr: NonNull<u8>) -> Option<H> {
        let mut maybe_block = Some(self.first_block());
        while let Some(block) = maybe_block
            && block.get_data() != ptr
        {
            let next = &self.next_header(&block);
            maybe_block = *next;
        }

        maybe_block
    }

    fn number_of_blocks(&self) -> usize {
        let mut c = 0;
        let mut head = Some(self.first_block());
        while let Some(head_ptr) = head {
            c += 1;
            head = self.next_header(&head_ptr);
        }

        c
    }
}

impl<'a, A, H> WithPageSize for ListAllocator<'a, A, H>
where
    A: PageAllocator + WithPageSize,
    H: InlineHeader,
{
    /// Creates a new contiguous list allocator
    /// Manually allocates its own page_allocator with a given page size (not recommended)
    ///
    /// # Safety
    /// Panics if:
    /// - The first page cannot be allocated
    ///
    /// # Usage
    /// ```rust
    /// use yerba::{array_page_allocator::ArrayPageAllocator, list_allocator::ListAllocator, with_page_size::WithPageSize};
    /// let allocator = ListAllocator::<ArrayPageAllocator>::with_page_size(4096);
    /// ```
    fn with_page_size(page_size: usize) -> Self {
        let allocator = A::with_page_size(page_size);
        Self::with_allocator(allocator)
    }
}

impl<'a, A, H> Default for ListAllocator<'a, A, H>
where
    A: PageAllocator + WithPageSize,
    H: InlineHeader,
{
    /// Creates a new contiguous list allocator
    /// Manually allocates its own page_allocator using the default page size (not recommended )
    ///
    /// # Safety
    /// Panics if:
    /// - The first page cannot be allocated
    ///
    /// # Usage
    /// ```rust
    /// use yerba::{array_page_allocator::ArrayPageAllocator, list_allocator::ListAllocator};
    /// let allocator = ListAllocator::<ArrayPageAllocator>::default();
    /// ```

    fn default() -> Self {
        Self::with_page_size(PAGE_SIZE)
    }
}

unsafe impl<'a, A, H> Allocator for ListAllocator<'a, A, H>
where
    A: PageAllocator,
    H: InlineHeader,
{
    /// Allocates a new block with capacity `size` in the allocator
    /// If a block is found whose size exceeds `size` by more than `size_of::<Header>()`, it will be split into two blocks
    /// and a pointer to the first of the headers will be returned
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if !H::is_valid_layout(&layout) {
            return Err(AllocError);
        }

        let size = layout.size();
        let align = layout.align();

        let mut header = self.find_empty_block(size, align)?;
        let data_ptr = header.get_data();

        let end_of_block = data_ptr.as_ptr().addr() + size;
        let top_of_buf = self.last_addr();
        if end_of_block > top_of_buf {
            return Err(AllocError);
        }

        header.mark_used();
        header.try_split_allocated_block(size, self.last_addr());

        Ok(NonNull::slice_from_raw_parts(data_ptr, size))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, _layout: Layout) {
        let mut block = self.find_ptr_block(ptr);
        match block {
            Some(ref mut block_ptr) => {
                block_ptr.mark_free();
                block_ptr.set_offset(0);
            }
            None => return,
        }
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        layout: Layout,
        new_layout: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let new_size = new_layout.size();

        // First look forward for adjacent free blocks
        let mut header_ptr = self.find_ptr_block(ptr).ok_or_else(|| AllocError)?;
        header_ptr.mark_free();
        let mut frontier_ptr = self.next_header(&header_ptr);
        let mut acc_size = header_ptr.size();
        while let Some(ref frontier) = frontier_ptr
            && acc_size < new_size
        {
            if frontier.used() {
                break;
            }

            acc_size += frontier.size() + frontier.get_offset() + H::header_size();
            if acc_size >= new_size {
                let alignment_offset = header_ptr.align_offset(layout.align());
                unsafe {
                    header_ptr.set_offset(alignment_offset);
                    return Ok(NonNull::slice_from_raw_parts(
                        header_ptr.get_data().add(alignment_offset),
                        new_size,
                    ));
                }
            }
            unsafe { frontier_ptr = Some(H::from(frontier.add(1))) };
        }
        if acc_size > new_size {
            return Ok(NonNull::slice_from_raw_parts(ptr, new_size));
        }
        let alignment_offset = header_ptr.align_offset(layout.align());

        // Then start at the first block and check for available adjacent blocks again
        let mut anchor_ptr = Some(self.first_block());
        while let Some(anchor) = anchor_ptr {
            if anchor.used() {
                anchor_ptr = self.next_header(&anchor);
                continue;
            }

            acc_size = anchor.size();
            frontier_ptr = anchor_ptr;
            while let Some(frontier) = frontier_ptr
                && acc_size < new_size
            {
                if frontier.used() {
                    anchor_ptr = self.next_header(&frontier);
                    break;
                }

                acc_size += frontier.size() + frontier.get_offset() + H::header_size();

                if acc_size >= new_size {
                    unsafe {
                        header_ptr.set_offset(alignment_offset);
                        return Ok(NonNull::slice_from_raw_parts(
                            header_ptr.get_data().add(alignment_offset),
                            new_size,
                        ));
                    }
                }
                unsafe { frontier_ptr = Some(H::from(frontier.add(1))) };
            }
        }

        unsafe {
            self.add_page(new_size)?;

            let header_ptr = frontier_ptr.unwrap();

            // Ideally they don't request more than a page
            while new_size > header_ptr.size() {
                self.add_page(new_size)?;
                header_ptr.write_bytes(0, H::header_size());
            }
        }

        let data_ptr = header_ptr.get_data();
        let alignment_offset = data_ptr.align_offset(layout.align());
        let data_ptr =
            unsafe { NonNull::slice_from_raw_parts(data_ptr.add(alignment_offset), new_size) };

        if new_size + alignment_offset > header_ptr.size() {
            return Err(AllocError);
        }

        return Ok(data_ptr);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use core::alloc::Layout;
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
