use core::ffi::c_void;
use core::ptr::{self};
use libc::{
    self, __errno_location, MAP_ANONYMOUS, MAP_FAILED, MAP_FIXED, MAP_NORESERVE, MAP_PRIVATE,
    MAP_SHARED, PROT_READ, PROT_WRITE, munmap,
};
use std::cell::UnsafeCell;
use std::ptr::slice_from_raw_parts_mut;
use std::sync::atomic::{AtomicU8, AtomicUsize};

use crate::page_allocator_trait::PageAllocator;

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
pub struct YerbaPageAllocator {
    page_size: usize,
    // Represents the number of page_arrays we've allocated
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

impl PageAllocator for YerbaPageAllocator {
    fn new(page_size: usize) -> Self {
        unsafe {
            // Create the underlying block for storing pointers to arrays of blocks and the sizes
            // of those arrays
            // This will be of type `*mut [PageArray]`
            let page_ptr_ptr = libc::mmap(
                ptr::null_mut(),
                12, // Arbitrary, means 12*12 pages can be allocated
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE,
                -1,
                0,
            );
            if page_ptr_ptr == MAP_FAILED {
                panic!("Failed to reserve the underlying pointer array")
            }
            let underlying_ptr_array =
                slice_from_raw_parts_mut(page_ptr_ptr as *mut u8, page_size) as *mut [PageArray];

            // The first pointer to the array of page pointers (PageArray)
            // This will be of type `*mut Page`
            let base_page_ptr = libc::mmap(
                ptr::null_mut(),
                page_size * 12,
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE,
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
            underlying_ptr_array
                .cast::<PageArray>()
                .write(initial_page_array);

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

            YerbaPageAllocator {
                page_size,
                page_array_count: AtomicU8::from(1),
                page_block_ptr_array: underlying_ptr_array,
            }
        }
    }
    unsafe fn request_page(&self) -> *mut u8 {
        let page_array_count = self.page_array_count();
        assert_eq!(page_array_count, 1); // testing

        let curr_page_array_ptr =
            (self.page_block_ptr_array as *mut PageArray).wrapping_add(page_array_count - 1);
        assert!(!curr_page_array_ptr.is_null());

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
            println!("here");
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

    unsafe fn request_page_zeroed(&self) -> *mut u8 {
        unsafe {
            let address = self.request_page();
            if address.is_null() {
                return ptr::null_mut();
            }
            address.write_bytes(0, self.page_size);

            address
        }
    }

    unsafe fn relinquish_page(&self, ptr: *mut u8) {
        // TODO Figure out how to effectively clear the pointer to this underlying data in our
        // pointer array
        unsafe { munmap(ptr.cast::<c_void>(), self.page_size) };
    }
}

impl YerbaPageAllocator {
    fn page_array_count(&self) -> usize {
        self.page_array_count
            .load(std::sync::atomic::Ordering::Relaxed)
            .into()
    }

    fn to_page_ptr<T: ?Sized>(&self, ptr: *mut T) -> *mut Page {
        slice_from_raw_parts_mut(ptr.cast::<u8>(), self.page_size) as *mut Page
    }
}

impl Default for YerbaPageAllocator {
    fn default() -> Self {
        Self::new(DEFAULT_PAGE_SIZE)
    }
}
