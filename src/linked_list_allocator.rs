use core::{
    alloc::GlobalAlloc,
    cell::UnsafeCell,
    ffi::c_void,
    fmt::Pointer,
    ops::Deref,
    ptr::{self, slice_from_raw_parts_mut},
    sync::atomic::AtomicU8,
};

use libc::{
    __errno_location, ENOMEM, MAP_ANONYMOUS, MAP_FAILED, MAP_FIXED, MAP_PRIVATE, MAP_SHARED,
    PROT_READ, PROT_WRITE, arpd_request,
};

const PAGE_SIZE: usize = 4096;
const MIN_BLOCK_SIZE: usize = 8;
const MAX_ALIGN: usize = 32;
const MIN_ALIGN: usize = 2;

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
            let old_break = libc::sbrk(0);
            let mem_ptr = libc::mmap(
                old_break,
                PAGE_SIZE,
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED,
                -1,
                0,
            );
            if mem_ptr == MAP_FAILED {
                panic!("Failed to increment program break");
            }
            assert_eq!(old_break, mem_ptr);

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
            unsafe {
                header_ptr.write_bytes(0, 1);
            }
            return HeaderPtr::null();
        }
        if header_ptr.get_offset() + header_ptr.size() + header_ptr.addr()
            > self.buf_ptr().addr() + PAGE_SIZE
        {
            return HeaderPtr::null();
        }
        unsafe {
            header_ptr
                .byte_add(header_ptr.get_offset() + header_ptr.size())
                .into()
        }
    }

    // fn next_empty_block(&self, header_ptr: &HeaderPtr) -> HeaderPtr {
    //     if header_ptr.size() == 0 {
    //         unsafe {
    //             header_ptr.write_bytes(0, 1);
    //         }
    //         return HeaderPtr::null();
    //     }
    //     if header_ptr.get_offset() + header_ptr.size() + header_ptr.addr()
    //         > self.buf_ptr().addr() + PAGE_SIZE
    //     {
    //         return HeaderPtr::null();
    //     }
    //     unsafe {
    //         let mut next: HeaderPtr = header_ptr
    //             .byte_add(header_ptr.get_offset() + header_ptr.size())
    //             .into();
    //         if next.is_null() {
    //             return HeaderPtr::null();
    //         }
    //         if next.used() {
    //             next.set(&self.next_empty_block(&next));
    //         }
    //
    //         next
    //     }
    // }

    /// Gets the next block in the array, even if it's not initialized
    /// Returns null if out of owned range
    fn next_header_unchecked(&self, header_ptr: &HeaderPtr) -> HeaderPtr {
        let pages: usize = self.pages.load(std::sync::atomic::Ordering::Relaxed).into();
        if header_ptr.get_offset() + header_ptr.size() + header_ptr.addr()
            > self.buf_ptr().addr() + PAGE_SIZE * pages
        {
            return HeaderPtr::null();
        }
        unsafe {
            header_ptr
                .byte_add(size_of::<Header>() + header_ptr.get_offset() + header_ptr.size())
                .into()
        }
    }

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
                    let merged_size = header_ptr.size() + last_header_ptr.size();
                    let fits_with_merge = merged_size >= required_size;

                    if fits_with_merge {
                        let data_ptr = last_header_ptr.add(1);
                        let alignment_offset = data_ptr.align_offset(align);
                        if alignment_offset == usize::MAX {
                            return HeaderPtr::null();
                        }

                        last_header_ptr.set_offset(alignment_offset);
                        last_header_ptr.add_size(
                            header_ptr.size() + header_ptr.get_offset() + size_of::<Header>(),
                        );

                        header_ptr.write_bytes(0, size_of::<Header>());
                        header_ptr = last_header_ptr;

                        break;
                    }
                }

                last_header_ptr.set(&header_ptr);
                let next_block = &self.next_header(&header_ptr);
                // println!("{}", next_block.addr());
                if next_block.is_null() {
                    self.request_new_page();
                    let header = Header::new(size, alignment_offset);
                    let new_top_header = Header::new(
                        PAGE_SIZE - size_of::<Header>() * 2 - alignment_offset - size,
                        0,
                    );
                    let header_ptr =
                        HeaderPtr::new(slice_from_raw_parts_mut(last_header_ptr.0, PAGE_SIZE));
                    header_ptr.write(header);
                    let top_header_ptr = self.next_header_unchecked(&header_ptr);
                    top_header_ptr.write(new_top_header)
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
        unsafe { self.buf_ptr().add(PAGE_SIZE).addr() }
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
        let old_top = self.last_addr();
        let prog_brk = unsafe {
            libc::mmap(
                old_top as *mut c_void,
                PAGE_SIZE,
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED,
                -1,
                0,
            )
        };
        if prog_brk == MAP_FAILED {
            panic!("Failed to allocate new page");
            // return HeaderPtr::null();
        }
        assert_eq!(prog_brk.addr(), old_top);

        let _ = self
            .pages
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }

    fn free_allocator(self) {
        let pages = self.pages.load(core::sync::atomic::Ordering::Relaxed) as usize;
        unsafe {
            self.buf.cast::<u8>().write_bytes(0, PAGE_SIZE * pages);
            libc::munmap(self.buf.cast::<c_void>(), PAGE_SIZE * pages);
            // libc::brk(self.buf.cast::<c_void>()); // .byte_sub(PAGE_SIZE * pages).cast::<c_void>()
            // if *__errno_location() == ENOMEM {
            //     panic!("Failed to increment program break");
            // }
        };
    }
}

unsafe impl GlobalAlloc for LinkedListAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();
        if align > MAX_ALIGN {
            return ptr::null_mut();
        }

        let mut block = self.find_empty_block(size, align);
        if block.is_null() {
            return ptr::null_mut();
        }
        let data_ptr = block.get_data();

        let end_of_block = data_ptr.addr() + size;
        let top_of_buf = self.last_addr();
        if end_of_block > top_of_buf {
            return ptr::null_mut();
        }

        block.mark_used();

        let block_next_ptr = self.next_header_unchecked(&block);

        if block.size() > size_of::<Header>() + size
            && (block_next_ptr.addr() + size_of::<Header>()) < self.buf_ptr().addr() + PAGE_SIZE
        {
            let new_block_size = block.size() - size_of::<Header>() - size;
            block.set_size(size);
            let new_block = Header {
                size: new_block_size,
                offset: 0,
            };

            unsafe {
                block_next_ptr.write(new_block);
            }
        }

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
        frontier
        // Ideally they don't request more than a page
        while new_size > header.size() {
            self.request_new_page();
            unsafe { top.write_bytes(0, size_of::<Header>()) };
        }

        let data_ptr = header.get_data();
        let alignment_offset = data_ptr.align_offset(layout.align());
        let data_ptr = unsafe { data_ptr.add(alignment_offset) };

        if new_size + alignment_offset > header.size() {
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
