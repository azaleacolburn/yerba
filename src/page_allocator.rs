use core::alloc::{self, AllocError, GlobalAlloc, Layout};
use core::cmp::{self, max};
use core::ffi::{self, c_void};
use core::ptr::{self, NonNull, null, null_mut};
use libc::{
    self, MAP_ANONYMOUS, MAP_FAILED, MAP_FIXED, MAP_NORESERVE, MAP_PRIVATE, MAP_SHARED, PROT_READ,
    PROT_WRITE, mmap, munmap, sbrk,
};
use std::cell::UnsafeCell;
use std::ptr::slice_from_raw_parts_mut;
use std::sync::atomic::{AtomicU8, AtomicUsize};

const DEFAULT_PAGE_SIZE: usize = 4096;

type Page = UnsafeCell<[u8]>;

/// An allocator for managing entire pages of memory
/// Intended to be used by higher level allocators to abstact away
/// memory requests to the operating system
///
/// The pages are not guaranteed to be contiguous to each other, nor are they guaranteed
/// Unless a page larger than the default size, in which case it will be allocated
///
/// Pages are not guaranteed to be contiguous with respect to each other
/// The allocator will attempt to make them contiguous, however, if the allocation of a contiguous
/// block with mmap(fixed) fails, a new base of contiguous pages will be allocated
///
///
pub struct PageAllocator {
    page_size: usize,
    // If we allocate more pages then this wtf
    page_array_count: AtomicU8,
    page_block_ptr_array: *mut [PageArray],
}

struct PageArray {
    // Secretly, this is actually an array of pages
    // This is to avoid using an extra usize of space, since we functionally need both
    // the capacity (which is fixed the `PageAllocator::page_size`) and the length (which is the variable `page_count`)
    //
    // If `pages` where of type `*mut [Page]`, we would also have to store the page_size as the
    // wide pointer's provenance, which we're already storing in `PageAllocator`, our parent struct
    pages: *mut Page,
    page_count: AtomicUsize,
}

impl PageArray {
    fn last_addr(&self, page_size: usize) -> *mut c_void {
        unsafe {
            let first_page_ptr = self.pages.cast::<*mut Page>().read();
            first_page_ptr
                .byte_add(page_size * self.page_count())
                .cast()
        }
    }

    fn page_count(&self) -> usize {
        self.page_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn set_page_count(&self, n: impl Into<usize>) -> usize {
        let n = n.into();
        self.page_count
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |_| Some(n),
            )
            .unwrap()
    }
}

impl PageAllocator {
    fn new(page_size: usize) -> Self {
        unsafe {
            // Create the underlying block for storing pointers to arrays of blocks and the sizes
            // of those arrays
            // This will be of type `*mut [PageArray]`
            let page_ptr_ptr = libc::mmap(
                ptr::null_mut(),
                page_size,
                PROT_READ | PROT_WRITE,
                MAP_NORESERVE | MAP_ANONYMOUS,
                -1,
                0,
            );
            if page_ptr_ptr == MAP_FAILED {
                panic!("Failed to allocate blocks buffer")
            }
            let underlying_ptr_array =
                slice_from_raw_parts_mut(page_ptr_ptr as *mut u8, page_size) as *mut [PageArray];

            // The first pointer to the array of page pointers (PageArray)
            // This will be of type `*mut Page`
            let base_page_ptr = libc::mmap(
                ptr::null_mut(),
                page_size * 12,
                PROT_READ | PROT_WRITE,
                MAP_NORESERVE | MAP_ANONYMOUS | MAP_SHARED,
                -1,
                0,
            );
            if base_page_ptr == MAP_FAILED {
                panic!("Failed to reserve initial page array");
            }

            let initial_page_array = PageArray {
                // This is to establish provenance on our `UnsafeCell<[u8]>`
                // In the future we might not do it this way
                pages: slice_from_raw_parts_mut(base_page_ptr, page_size) as *mut Page,
                page_count: AtomicUsize::from(1),
            };

            let first_block_ptr = libc::mmap(
                base_page_ptr,
                page_size,
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED,
                -1,
                0,
            );
            if first_block_ptr == MAP_FAILED {
                panic!("Failed to allocate first page");
            }
            assert_eq!(first_block_ptr, base_page_ptr);

            underlying_ptr_array
                .cast::<PageArray>()
                .write(initial_page_array);

            PageAllocator {
                page_size,
                page_array_count: AtomicU8::from(0),
                page_block_ptr_array: underlying_ptr_array,
            }
        }
    }

    fn page_array_count(&self) -> usize {
        self.page_array_count
            .load(std::sync::atomic::Ordering::Relaxed)
            .into()
    }

    fn to_page_ptr<T: ?Sized>(&self, ptr: *mut T) -> *mut Page {
        slice_from_raw_parts_mut(ptr.cast::<u8>(), self.page_size) as *mut Page
    }
}

impl Default for PageAllocator {
    fn default() -> Self {
        Self::new(DEFAULT_PAGE_SIZE)
    }
}

// Decide if implementing GlobalAlloc is the right solution for the page allocation system (I vote
// no)
unsafe impl GlobalAlloc for PageAllocator {
    unsafe fn alloc(&self, _layout: alloc::Layout) -> *mut u8 {
        let page_array_count = self.page_array_count();
        let curr_page_array_ptr = (self.page_block_ptr_array as *mut PageArray)
            .wrapping_byte_add(page_array_count * self.page_size);

        let curr_page_array = unsafe { curr_page_array_ptr.read() };
        let last_page_addr = curr_page_array.last_addr(self.page_size);

        let page = unsafe {
            libc::mmap(
                last_page_addr,
                self.page_size,
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED,
                -1,
                0,
            )
        };
        if page == MAP_FAILED {
            let base_page = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    self.page_size,
                    PROT_READ | PROT_WRITE,
                    MAP_ANONYMOUS | MAP_PRIVATE,
                    -1,
                    0,
                )
            };
            if base_page == MAP_FAILED {
                return ptr::null_mut();
            }
            let new_page_array = PageArray {
                pages: self.to_page_ptr(base_page),
                page_count: AtomicUsize::from(1),
            };
            unsafe {
                // Write the address of the new base page array to the page array buffer
                // Remeber that this extra layer of indirection is necessary for keeping pages
                // contiguous whenever possible
                self.page_block_ptr_array
                    .cast::<PageArray>()
                    .add(page_array_count)
                    .write(new_page_array)
            }
        } else {
            unsafe {
                (*curr_page_array_ptr)
                    .page_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        page.cast::<u8>()
    }

    unsafe fn alloc_zeroed(&self, layout: alloc::Layout) -> *mut u8 {
        let size = layout.size();
        let address = unsafe { self.alloc(layout) };
        (0..size).for_each(|i| unsafe { address.add(i).write(0) });

        address
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: alloc::Layout) {
        let size = layout.size();
        unsafe { munmap(ptr.cast::<c_void>(), size) };
    }

    // NOTE The concept of reallocating a page is a bit silly tbh
    // unsafe fn realloc(&self, ptr: *mut u8, old_layout: alloc::Layout, new_size: usize) -> *mut u8 {
    //     let layout = Layout::from_size_align(new_size, old_layout.align())
    //         .expect("Layout from alignment and new size failed");
    //
    //     let new_ptr = unsafe { self.alloc(layout) };
    //     (0..layout.size()).for_each(|i| unsafe { new_ptr.add(i).write(ptr.add(i).read()) });
    //
    //     unsafe { munmap(ptr.cast::<c_void>(), old_layout.size()) };
    //
    //     new_ptr
    // }
}
