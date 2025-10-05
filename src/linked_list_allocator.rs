use core::{
    alloc::GlobalAlloc,
    cell::UnsafeCell,
    ffi::c_void,
    ops::Deref,
    ptr::{self, slice_from_raw_parts_mut},
    sync::atomic::AtomicU8,
};
use std::alloc::Layout;

use libc::{
    MAP_ANONYMOUS, MAP_FAILED, MAP_FIXED, MAP_NORESERVE, MAP_PRIVATE, MAP_SHARED, PROT_READ,
    PROT_WRITE,
};

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
        Header::new(PAGE_SIZE - size_of::<Header>(), 0)
    }
}

impl Header {
    pub fn new(size: usize, offset: usize) -> Header {
        Header { size, offset }
    }
}

struct HeaderPtr(*mut Header);

impl HeaderPtr {
    pub fn new<T: ?Sized>(ptr: *mut T) -> Self {
        if ptr.is_null() {
            panic!("Tried to create HeaderPtr from null ptr, use HeaderPtr::null() instead")
        }
        Self(ptr.cast::<Header>())
    }
    pub fn null() -> HeaderPtr {
        HeaderPtr(ptr::null_mut())
    }

    pub fn get_offset(&self) -> usize {
        unsafe { (*self.0).offset & (0 as usize) << (size_of::<usize>() * 8 - 1) }
    }

    pub fn set_offset(&mut self, offset: usize) {
        let used: bool = self.used();
        unsafe {
            (*self.0).offset = offset;
        }
        self.set_used(used);
    }

    pub fn used(&self) -> bool {
        // Seems to be a bit faster or the same as bitshifting
        unsafe { (*self.0).offset.reverse_bits() & 1 == 1 }
    }

    fn set_used(&mut self, used: bool) {
        unsafe {
            let k = size_of::<usize>() * 8 - 1;
            (*self.0).offset &= 0 << k;
            (*self.0).offset &= (used as usize) << k;
        }
    }

    pub fn free(&mut self) {
        self.set_used(false)
    }

    pub fn mark_used(&mut self) {
        self.set_used(true)
    }

    pub fn size(&self) -> usize {
        unsafe { (*self.0).size }
    }

    pub fn add_size(&self, size: usize) {
        unsafe { (*self.0).size += size }
    }

    pub fn set_size(&self, size: usize) {
        unsafe { (*self.0).size = size }
    }

    pub fn set(&mut self, ptr: &HeaderPtr) {
        self.0 = ptr.0
    }

    fn get_data(&self) -> *mut u8 {
        let offset = self.get_offset();
        unsafe { self.add(1).byte_add(offset).cast::<u8>() as *mut u8 }
    }

    fn last_addr(&self) -> usize {
        self.addr() + size_of::<Header>() + self.get_offset() + self.size()
    }

    unsafe fn next_unchecked(&self) -> HeaderPtr {
        unsafe {
            self.byte_add(size_of::<Header>() + self.get_offset() + self.size())
                .into()
        }
    }

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

    fn split_allocated_block(&self, next_header: &HeaderPtr, size: usize) {}
}

impl Deref for HeaderPtr {
    type Target = *mut Header;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<*mut Header> for HeaderPtr {
    fn from(value: *mut Header) -> Self {
        HeaderPtr(value)
    }
}

// Headers are inlined to the buffer
// Only allocates a single arena and returns a null pointer for allocations past that
// Allows the arbitrary allocation, deallocation, and reallocation of any block
// Will merge empty blocks when necessary to fit new allocations
struct LinkedListAllocator {
    buf: *mut UnsafeCell<[u8]>,
    pages: AtomicU8,
}

impl LinkedListAllocator {
    pub fn new() -> Self {
        const {
            let header_size = size_of::<Header>();
            assert!(header_size < PAGE_SIZE);
            assert!(header_size % 8 == 0)
        }
        let head = Header::default();

        unsafe {
            let base_addr = libc::mmap(
                ptr::null_mut(),
                PAGE_SIZE * 12,
                PROT_READ | PROT_WRITE,
                MAP_NORESERVE | MAP_ANONYMOUS | MAP_SHARED,
                -1,
                0,
            );
            if base_addr == MAP_FAILED {
                panic!("Failed to reserve initial page array");
            }
            let mem_ptr = libc::mmap(
                base_addr,
                PAGE_SIZE,
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED,
                -1,
                0,
            );
            if mem_ptr == MAP_FAILED {
                panic!("Failed to allocate first page");
            }
            assert_eq!(mem_ptr, base_addr);

            let buf =
                slice_from_raw_parts_mut(mem_ptr as *mut u8, PAGE_SIZE) as *mut UnsafeCell<[u8]>;
            buf.cast::<Header>().write(head);

            Self {
                buf,
                pages: AtomicU8::new(1),
            }
        }
    }

    fn next_header(&self, header_ptr: &HeaderPtr) -> HeaderPtr {
        if header_ptr.size() == 0 {
            panic!("Should not have zero sized headers")
        }
        if header_ptr.last_addr() >= self.last_addr() {
            return HeaderPtr::null();
        }
        unsafe { header_ptr.next_unchecked() }
    }

    /// Requests a new page to accommodate a new block headed by `header_ptr` of size `size`
    /// # Args
    /// - `header_ptr`: the header pointer to be ultimately returned
    /// - `size`: the requested size of the block the header pointer will represent
    /// - `alignment_offset`: the calculated offset to be added to
    ///     the `data_ptr` that `header_ptr` represents, to align it to `T`, where `data_ptr`
    ///     is of type `*mut T`
    /// # Safety
    /// - Panics if header is null
    /// - Panics if twelve or more pages have already been allocated (subject to change)
    fn add_page(&self, header_ptr: &HeaderPtr, size: usize, alignment_offset: usize) {
        assert!(!header_ptr.is_null());
        self.request_new_page();

        let remaining_size = self.last_addr()
            - size_of::<Header>() * 2
            - alignment_offset
            - size
            - self.buf_ptr().addr();

        let header = Header::new(size, alignment_offset);
        let header_ptr = HeaderPtr::new(slice_from_raw_parts_mut(header_ptr.0, size));
        unsafe {
            header_ptr.write(header);
        }

        let new_top_header = Header::new(remaining_size, 0);
        let top_header_ptr = self.next_header(&header_ptr);
        assert!(!top_header_ptr.is_null());
        unsafe {
            top_header_ptr.write(new_top_header);
        }
    }

    /// Gets the next block in the array, even if it's not initialized
    ///
    /// # Returns
    /// - The first empty header pointer that accomodates `size` in the allocator.
    /// - A null pointer if unable to create an offset that aligns data pointer to `align`
    ///
    fn find_empty_block(&self, size: usize, align: usize) -> HeaderPtr {
        let mut last_header_ptr = HeaderPtr::null();
        let mut header_ptr = self.first_block();

        while !header_ptr.is_null() {
            unsafe {
                if header_ptr.used() {
                    last_header_ptr.set(&header_ptr);
                    let next_block = &self.next_header(&header_ptr);
                    header_ptr.set(next_block);

                    continue;
                }

                // We don't actually use this pointer again, it's just for calculating the offset
                let data_ptr = header_ptr.add(1).cast::<u8>();
                let alignment_offset = data_ptr.align_offset(align);
                if alignment_offset == usize::MAX {
                    return HeaderPtr::null();
                }

                let required_size = size + alignment_offset;
                let fits = header_ptr.size() >= required_size;

                // We've found a block that fits
                if fits {
                    header_ptr.set_offset(alignment_offset);

                    break;
                }

                // We've found a pair of free blocks that can be merged to fit
                let mergeable = !last_header_ptr.is_null() && !last_header_ptr.used();
                if mergeable {
                    let merge_failed =
                        !header_ptr.merge_block(&mut last_header_ptr, required_size, align);
                    if merge_failed {
                        return HeaderPtr::null();
                    }

                    break;
                }

                last_header_ptr.set(&header_ptr);
                let next_block = &self.next_header(&header_ptr);
                if next_block.is_null() {
                    self.add_page(&header_ptr, size, alignment_offset);
                    break;
                }
                header_ptr.set(next_block);
            }
        }

        header_ptr
    }

    fn first_block(&self) -> HeaderPtr {
        HeaderPtr(self.buf_ptr() as *mut Header)
    }

    fn last_addr(&self) -> usize {
        let pages = self.pages.load(std::sync::atomic::Ordering::Relaxed) as usize;
        unsafe { self.buf_ptr().add(PAGE_SIZE * pages).addr() }
    }

    fn buf_ptr(&self) -> *mut u8 {
        unsafe { (*self.buf).get().cast() }
    }

    /// Finds the block representing the given data pointer
    fn find_ptr_block(&self, ptr: *mut u8) -> HeaderPtr {
        let mut block = self.first_block();
        while block.get_data() != ptr && !block.is_null() {
            block.set(&self.next_header(&block));
        }

        block
    }

    fn number_of_blocks(&self) -> usize {
        let mut c = 0;
        let mut head = self.first_block();
        while !head.is_null() {
            c += 1;
            head.set(&self.next_header(&head));
        }

        c
    }

    // Allocates a new page in memory and then returns the new top HeaderPtr
    // with provenance of PAGE_SIZE
    fn request_new_page(&self) {
        let pages = self
            .pages
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed) as usize
            - 1;

        unsafe {
            let base_addr = self.buf_ptr().cast::<c_void>().byte_add(PAGE_SIZE * pages);
            let prog_brk = libc::mmap(
                base_addr,
                PAGE_SIZE,
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED,
                -1,
                0,
            );

            if prog_brk == MAP_FAILED {
                panic!("Failed to allocate new page");
            }
            assert_eq!(prog_brk, base_addr);
        }
    }

    fn free_allocator(self) {
        let _pages = self.pages.load(core::sync::atomic::Ordering::Relaxed) as usize;
        unsafe {
            self.buf.cast::<u8>().write_bytes(0, MAX_BLOCK_SIZE);
            let success = libc::munmap(self.buf.cast::<c_void>(), MAX_BLOCK_SIZE);
            if success == -1 {
                panic!("Failed to unmap allocator memory");
            }
        };
    }

    /// Attempts to split the allocated block represented
    /// by `header`, into two blocks
    fn try_split_allocated_block(&self, header: &HeaderPtr, requested_size: usize) {
        let next_header = unsafe { header.next_unchecked() };
        if !self.can_split_allocated_block(&header, &next_header, requested_size) {
            return;
        }

        let new_block_size = header.size() - size_of::<Header>() - requested_size;
        header.set_size(requested_size);

        let new_header = Header {
            size: new_block_size,
            offset: 0,
        };
        unsafe {
            next_header.write(new_header);
        }
    }

    fn can_split_allocated_block(
        &self,
        header: &HeaderPtr,
        next_header: &HeaderPtr,
        size: usize,
    ) -> bool {
        let pages = self.pages.load(std::sync::atomic::Ordering::Relaxed) as usize;
        header.size() > size_of::<Header>() + size
            && (next_header.addr() + size_of::<Header>())
                < self.buf_ptr().addr() + PAGE_SIZE * pages
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

unsafe impl GlobalAlloc for LinkedListAllocator {
    /// Allocates a new block with capacity `size` in the allocator
    /// If a block is found whose size exceeds `size` by more than `size_of::<Header>()`, it will be split into two blocks
    /// and a pointer to the first of the headers will be returned
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if is_invalid_layout(&layout) {
            return ptr::null_mut();
        }

        let size = layout.size();
        let align = layout.align();

        let mut header = self.find_empty_block(size, align);
        if header.is_null() {
            return ptr::null_mut();
        }
        let data_ptr = header.get_data();

        let end_of_block = data_ptr.addr() + size;
        let top_of_buf = self.last_addr();
        if end_of_block > top_of_buf {
            return ptr::null_mut();
        }
        header.mark_used();

        self.try_split_allocated_block(&header, size);

        data_ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: core::alloc::Layout) {
        let mut block = self.find_ptr_block(ptr);

        block.free();
        block.set_offset(0);
    }

    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        // First look forward for adjacent free blocks
        let mut header_ptr = self.find_ptr_block(ptr);
        header_ptr.free();
        let mut frontier = self.next_header(&header_ptr);
        let mut acc_size = header_ptr.size();
        while acc_size < new_size && !frontier.is_null() {
            if frontier.used() {
                break;
            }

            acc_size += frontier.size() + frontier.get_offset() + size_of::<Header>();
            if acc_size >= new_size {
                let alignment_offset = header_ptr.align_offset(layout.align());
                unsafe {
                    header_ptr.set_offset(alignment_offset);
                    return header_ptr.get_data().add(alignment_offset);
                }
            }
            unsafe { frontier.set(&HeaderPtr(frontier.add(1))) };
        }
        if acc_size > new_size {
            return ptr;
        }
        // Then start at the first block and check for available adjacent blocks again
        let mut anchor = self.first_block();
        while !anchor.is_null() {
            if anchor.used() {
                anchor.set(&self.next_header(&anchor));
                continue;
            }

            acc_size = anchor.size();
            frontier.set(&anchor);
            while acc_size < new_size && !frontier.is_null() {
                if frontier.used() {
                    anchor.set(&self.next_header(&frontier));
                    assert!(!anchor.is_null());
                    break;
                }

                acc_size += frontier.size() + frontier.get_offset() + size_of::<Header>();

                if acc_size >= new_size {
                    let alignment_offset = header_ptr.align_offset(layout.align());
                    unsafe {
                        header_ptr.set_offset(alignment_offset);
                        return header_ptr.get_data().add(alignment_offset);
                    }
                }
                unsafe { frontier.set(&HeaderPtr(frontier.add(1))) };
            }
        }

        self.request_new_page();
        let header_ptr = frontier;
        // Ideally they don't request more than a page
        while new_size > header_ptr.size() {
            self.request_new_page();
            unsafe { header_ptr.write_bytes(0, size_of::<Header>()) };
        }

        let data_ptr = header_ptr.get_data();
        let alignment_offset = data_ptr.align_offset(layout.align());
        let data_ptr = unsafe { data_ptr.add(alignment_offset) };

        if new_size + alignment_offset > header_ptr.size() {
            return ptr::null_mut();
        }

        return data_ptr;
    }

    unsafe fn alloc_zeroed(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size();
        unsafe {
            let ptr = self.alloc(layout);
            if ptr.is_null() {
                return ptr::null_mut();
            }

            ptr.write_bytes(0, size);
            ptr
        }
    }
}

#[cfg(test)]
mod test {
    use core::alloc::Layout;

    use super::*;

    #[test]
    fn alloc_chunks() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let chunk = allocator.alloc(layout);
            assert!(!chunk.is_null());
            allocator.dealloc(chunk, layout);

            let one = allocator.alloc(layout);
            assert!(!one.is_null());

            let two = allocator.alloc(layout);
            assert!(!two.is_null());

            let three = allocator.alloc(layout);
            assert!(!three.is_null());

            allocator.dealloc(three, layout);
            allocator.dealloc(one, layout);
            allocator.dealloc(two, layout);
        }

        allocator.free_allocator();
    }

    #[test]
    fn overflow() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 5000]>();

        unsafe {
            let one = allocator.alloc(layout);
            assert!(!one.is_null());
            allocator.dealloc(one, layout);

            let two = allocator.alloc(layout);
            assert!(!two.is_null());
            allocator.dealloc(two, layout);
        }
        allocator.free_allocator();
    }

    #[test]
    fn zeroed() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc_zeroed(layout);
            assert!(!one.is_null());

            let two = allocator.alloc_zeroed(layout);
            assert!(!two.is_null());

            let two_sum: u8 = (0..16).into_iter().map(|i| *(two.wrapping_add(i))).sum();
            let one_sum: u8 = (0..16).into_iter().map(|i| *(one.wrapping_add(i))).sum();
            assert_eq!(two_sum, 0);
            assert_eq!(one_sum, 0);

            allocator.dealloc(two, layout);
            allocator.dealloc(one, layout);
        }

        allocator.free_allocator();
    }

    #[test]
    fn realloc() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc(layout);
            assert!(!one.is_null());

            let two = allocator.alloc(layout);
            assert!(!two.is_null());

            allocator.realloc(two, layout, 32);
            allocator.dealloc(one, layout);
            allocator.dealloc(two, Layout::new::<[u8; 32]>());
        }

        allocator.free_allocator();
    }

    #[test]
    fn merge() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 2000]>();

        unsafe {
            let one = allocator.alloc(layout);
            assert!(!one.is_null());
            allocator.dealloc(one, layout);

            let layout = Layout::new::<[u8; 3080]>();
            let two = allocator.alloc(layout);
            assert!(!two.is_null());
        }

        allocator.free_allocator();
    }
}
