use crate::array_page_allocator::ArrayPageAllocator;
use crate::newable::Newable;
use crate::page_allocator::PageAllocator;
use crate::util::to_non_null_slice;
use core::alloc::{AllocError, Allocator, Layout};
use core::cell::RefCell;
use core::marker::PhantomData;
use core::ptr::NonNull;
use core::{cell::UnsafeCell, ops::Deref, ptr::slice_from_raw_parts_mut};

const PAGE_SIZE: usize = 4096;
const MIN_BLOCK_SIZE: usize = 8;
const MAX_BLOCK_SIZE: usize = PAGE_SIZE * 12;
const MAX_ALIGN: usize = 32;
const MIN_ALIGN: usize = 1;

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
struct HeaderPtr(NonNull<Header>);

impl HeaderPtr {
    pub fn new<T: ?Sized>(ptr: *mut T) -> Self {
        if ptr.is_null() {
            panic!("Tried to create HeaderPtr from null ptr, use HeaderPtr::null() instead")
        }
        let non_null = unsafe { NonNull::new_unchecked(ptr.cast()) };
        Self(non_null)
    }

    pub fn get_offset(&self) -> usize {
        unsafe { (self.0.read()).offset & (0 as usize) << (size_of::<usize>() * 8 - 1) }
    }

    pub fn set_offset(&mut self, offset: usize) {
        let used: bool = self.used();
        unsafe {
            self.0.as_mut().offset = offset;
        }
        self.set_used(used);
    }

    pub fn used(&self) -> bool {
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

    pub fn free(&mut self) {
        self.set_used(false)
    }

    pub fn mark_used(&mut self) {
        self.set_used(true)
    }

    pub fn size(&self) -> usize {
        unsafe { self.0.read().size }
    }

    pub fn add_size(&mut self, size: usize) {
        unsafe { self.0.as_mut().size += size }
        // unsafe { (*self.0.write();) += size }
    }

    pub fn set_size(&mut self, size: usize) {
        unsafe { self.0.as_mut().size = size }
    }

    pub fn set(&mut self, ptr: &HeaderPtr) {
        self.0 = ptr.0
    }

    fn get_data(&self) -> *mut u8 {
        let offset = self.get_offset();
        unsafe { self.add(1).byte_add(offset).cast::<u8>().as_ptr() }
    }

    fn last_addr(&self) -> usize {
        usize::from(self.addr()) + size_of::<Header>() + self.get_offset() + self.size()
    }

    unsafe fn next_unchecked(&self) -> HeaderPtr {
        unsafe {
            self.byte_add(size_of::<Header>() + self.get_offset() + self.size())
                .into()
        }
    }

    /// Merges two consecutive memory blocks in the buffer
    fn merge_block(
        &mut self,
        last_header: &mut HeaderPtr,
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
            self.set(&last_header);
        }

        true
    }
}

impl Deref for HeaderPtr {
    type Target = NonNull<Header>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<NonNull<Header>> for HeaderPtr {
    fn from(value: NonNull<Header>) -> Self {
        HeaderPtr(value)
    }
}

// Headers are inlined to the buffer
// Only allocates a single arena and returns a null pointer for allocations past that
// Allows the arbitrary allocation, deallocation, and reallocation of any block
// Will merge empty blocks when necessary to fit new allocations
//
// NOTE Explicitly dropping is not important because
// all the underlying memory is deallocated by the page allocator
//
/// An allocator in the shape of a single contiguous buffer, capable of performing allocations, deallocations, and reallocations.
/// It automatically merges and splits free block when suitable.
///
/// # Use Cases
/// This is a versatile allocator with few limitations
/// ## Especially suitable for
/// - Lots of small-midsized allocations/reallocations
/// - Single threaded code
///
/// ## Limitations
/// - Fails to allocate/reallocate if there isn't enough space in the underlying buffer and the system call to
/// request more contiguous memory fails.
/// - Not suitable for large allocations/reallocations or multithreaded code
///
/// While plugging in a custom `PageAllocator` is possible, it is not recommended to use one that
/// plans to allocate more than one contiguous page, as this allocator could not utilize it fully.
/// This, of course, is not the case if you pass in a specific instance of a `PageAllocator` with
/// `ContiguousListAllocator::with_allocator`
///
/// # Usage
/// ## Direct Allocation
/// ```rust
/// #![feature(allocator_api)]
/// use yerba::{contiguous_list_allocator::ContiguousListAllocator, array_page_allocator::ArrayPageAllocator};
/// use core::alloc::{Allocator, Layout};
///
/// let test = 0;
/// let allocator = ContiguousListAllocator::<ArrayPageAllocator>::new();
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
/// use yerba::contiguous_list_allocator::ContiguousListAllocator;
///
/// let allocator = ContiguousListAllocator::new();
/// let mut chunk = Box::<[u8; 16], ContiguousListAllocator>::new_in([0; 16], allocator);
/// chunk[0] = 1;
/// ```
///
///
pub struct ContiguousListAllocator<'a, A = ArrayPageAllocator<'a>>
where
    A: PageAllocator,
{
    buf: *mut UnsafeCell<[u8]>,
    page_allocator: RefCell<A>,
    phantom: PhantomData<&'a ()>,
}

impl<'a, A> ContiguousListAllocator<'a, A>
where
    A: PageAllocator + Newable,
{
    /// Creates a new contiguous list allocator
    /// Manually allocates its own page_allocator (not recommended )
    ///
    /// # Safety
    /// Panics if:
    /// - The first page cannot be allocated
    ///
    /// # Usage
    /// ```rust
    /// use yerba::{array_page_allocator::ArrayPageAllocator, contiguous_list_allocator::ContiguousListAllocator};
    /// let allocator = ContiguousListAllocator::<ArrayPageAllocator>::new();
    /// ```
    pub fn new() -> Self {
        let page_allocator = A::new(PAGE_SIZE);
        Self::with_allocator(page_allocator)
    }
}

impl<'a, A: PageAllocator> ContiguousListAllocator<'a, A> {
    /// Creates a new contiguous list allocator with a given `PageAllocator` instance
    ///
    /// # Safety
    /// Panics if:
    /// - The first page cannot be allocated
    pub fn with_allocator(mut page_allocator: A) -> Self {
        const {
            let header_size = size_of::<Header>();
            assert!(header_size < PAGE_SIZE);
            assert!(header_size % 8 == 0)
        }

        let first_page_ptr = unsafe { page_allocator.request_page_zeroed() };
        if first_page_ptr.is_null() {
            panic!("Failed to allocate the first page");
        }

        let first_page_buf =
            slice_from_raw_parts_mut(first_page_ptr, PAGE_SIZE) as *mut UnsafeCell<[u8]>;

        let head = Header::default();
        unsafe {
            first_page_buf.cast::<Header>().write(head);
        }

        Self {
            buf: first_page_buf,
            page_allocator: RefCell::new(page_allocator),
            phantom: PhantomData,
        }
    }

    fn next_header(&self, header_ptr: &HeaderPtr) -> Option<HeaderPtr> {
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
            if new_page.is_null() || old_last_addr != new_page.addr() {
                return Err(AllocError);
            }

            header.set_size(initial_header_size + PAGE_SIZE);
            self.try_split_allocated_block(&mut header, size);

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

    fn last_block(&self) -> HeaderPtr {
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
    fn find_empty_block(&self, size: usize, align: usize) -> Result<HeaderPtr, AllocError> {
        let mut last_header_ptr: Option<HeaderPtr> = None;
        let mut curr_header_ptr: Option<HeaderPtr> = Some(self.first_block());

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

    fn first_block(&self) -> HeaderPtr {
        HeaderPtr::new(self.buf_ptr())
    }

    fn last_addr(&self) -> usize {
        let pages = self.page_allocator.borrow().get_pages_allocated();
        self.buf_ptr().wrapping_add(PAGE_SIZE * pages).addr()
    }

    fn buf_ptr(&self) -> *mut u8 {
        unsafe { (*self.buf).get().cast() }
    }

    /// Finds the block representing the given data pointer
    /// If it does not exist, null is returned instead
    fn find_ptr_block(&self, ptr: NonNull<u8>) -> Option<HeaderPtr> {
        let mut maybe_block = Some(self.first_block());
        while let Some(block) = maybe_block
            && block.get_data() != ptr.as_ptr()
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

    /// Attempts to split the allocated block represented
    /// by `header`, into two blocks, the first one of size `new_size`
    ///
    /// Does nothing if there isn't enough space to split the block
    /// Or if `new_size > header.size() + size_of::<Header>()`
    fn try_split_allocated_block(&self, header: &mut HeaderPtr, new_size: usize) {
        let next_header = unsafe { header.next_unchecked() };
        if !self.can_split_allocated_block(&header, &next_header, new_size) {
            return;
        }

        let second_block_size = header.size() - size_of::<Header>() - new_size;
        header.set_size(new_size);

        let new_header = Header::new(second_block_size);
        unsafe {
            next_header.write(new_header);
        }
    }

    fn can_split_allocated_block(
        &self,
        header: &HeaderPtr,
        next_header: &HeaderPtr,
        new_size: usize,
    ) -> bool {
        let space_for_new_block = header.size() > size_of::<Header>() + new_size;
        let within_buffer =
            (usize::from(next_header.addr()) + size_of::<Header>()) < self.last_addr();

        space_for_new_block && within_buffer
    }
}

impl<'a, A> Default for ContiguousListAllocator<'a, A>
where
    A: PageAllocator + Newable,
{
    fn default() -> Self {
        Self::new()
    }
}

fn is_invalid_layout(&layout: &Layout) -> bool {
    let align = layout.align();
    let size = layout.size();
    align > MAX_ALIGN
        || align < MIN_ALIGN
        || size < MIN_BLOCK_SIZE
        || size + size_of::<Header>() > MAX_BLOCK_SIZE
}

unsafe impl<'a, A> Allocator for ContiguousListAllocator<'a, A>
where
    A: PageAllocator,
{
    /// Allocates a new block with capacity `size` in the allocator
    /// If a block is found whose size exceeds `size` by more than `size_of::<Header>()`, it will be split into two blocks
    /// and a pointer to the first of the headers will be returned
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if is_invalid_layout(&layout) {
            return Err(AllocError);
        }

        let size = layout.size();
        let align = layout.align();

        let mut header = self.find_empty_block(size, align)?;
        let data_ptr = header.get_data();

        let end_of_block = data_ptr.addr() + size;
        let top_of_buf = self.last_addr();
        if end_of_block > top_of_buf {
            return Err(AllocError);
        }

        header.mark_used();
        self.try_split_allocated_block(&mut header, size);

        Ok(to_non_null_slice(data_ptr, size)?)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, _layout: Layout) {
        let mut block = self.find_ptr_block(ptr);
        match block {
            Some(ref mut block_ptr) => {
                block_ptr.free();
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
        header_ptr.free();
        let mut frontier_ptr = self.next_header(&header_ptr);
        let mut acc_size = header_ptr.size();
        while let Some(frontier) = frontier_ptr
            && acc_size < new_size
        {
            if frontier.used() {
                break;
            }

            acc_size += frontier.size() + frontier.get_offset() + size_of::<Header>();
            if acc_size >= new_size {
                let alignment_offset = header_ptr.align_offset(layout.align());
                unsafe {
                    header_ptr.set_offset(alignment_offset);
                    return Ok(to_non_null_slice(
                        header_ptr.get_data().add(alignment_offset),
                        new_size,
                    )?);
                }
            }
            unsafe { frontier_ptr = Some(HeaderPtr(frontier.add(1))) };
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
                    // assert!(!anchor.is_null());
                    break;
                }

                acc_size += frontier.size() + frontier.get_offset() + size_of::<Header>();

                if acc_size >= new_size {
                    unsafe {
                        header_ptr.set_offset(alignment_offset);
                        return to_non_null_slice(
                            header_ptr.get_data().add(alignment_offset),
                            new_size,
                        );
                    }
                }
                unsafe { frontier_ptr = Some(HeaderPtr(frontier.add(1))) };
            }
        }

        unsafe {
            self.add_page(new_size)?;

            let header_ptr = frontier_ptr.unwrap();
            // Ideally they don't request more than a page
            while new_size > header_ptr.size() {
                self.add_page(new_size)?;
                header_ptr.write_bytes(0, size_of::<Header>());
            }
        }

        let data_ptr = header_ptr.get_data();
        let alignment_offset = data_ptr.align_offset(layout.align());
        let data_ptr = unsafe { to_non_null_slice(data_ptr.add(alignment_offset), new_size)? };

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

    #[test]
    fn alloc_chunks() {
        let allocator = ContiguousListAllocator::<ArrayPageAllocator>::new();
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
        let allocator = ContiguousListAllocator::<ArrayPageAllocator>::new();
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
        let allocator = ContiguousListAllocator::<ArrayPageAllocator>::new();
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
        let allocator = ContiguousListAllocator::<ArrayPageAllocator>::new();
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
        let allocator = ContiguousListAllocator::<ArrayPageAllocator>::new();
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
            ContiguousListAllocator::<&mut ArrayPageAllocator>::with_allocator(&mut page_allocator);
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
        let allocator = ContiguousListAllocator::new();
        let mut chunk = Box::<[u8; 16], ContiguousListAllocator>::new_in([0; 16], allocator);
        chunk[0] = 1;
    }
}
