use crate::page_allocator_trait::PageAllocator;
use core::ffi::c_void;
use core::ptr::{self};
use libc::{
    self, MAP_ANONYMOUS, MAP_FAILED, MAP_FIXED, MAP_PRIVATE, PROT_READ, PROT_WRITE, munmap,
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
    pages_loaned: AtomicUsize,
    pages_allocated: AtomicUsize,
}

impl PageArray {
    fn last_addr(&self, page_size: usize) -> *mut c_void {
        unsafe {
            let first_page_ptr = self.pages.cast::<*mut Page>();
            first_page_ptr
                .byte_add(page_size * self.page_capacity() - 1)
                .cast()
        }
    }

    fn page_capacity(&self) -> usize {
        self.pages_allocated
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    fn pages_loaned(&self) -> usize {
        self.pages_loaned.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn set_loaned_page_count(&self, n: impl Into<usize>) -> usize {
        let n = n.into();
        self.pages_allocated
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |_| Some(n),
            )
            .unwrap()
    }

    fn decrement_loaned_page_count(&self) {
        self.pages_allocated
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn increment_allocated_page_count(&self) -> usize {
        self.pages_allocated
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

fn map_arbitrary(size: usize) -> Result<*mut c_void, ()> {
    let ptr = unsafe {
        libc::mmap(
            ptr::null_mut(),
            size,
            PROT_READ | PROT_WRITE,
            MAP_ANONYMOUS | MAP_PRIVATE,
            -1,
            0,
        )
    };

    if ptr == MAP_FAILED {
        return Err(());
    };

    Ok(ptr)
}

fn map_fixed<T>(base: *mut T, size: usize) -> Result<*mut c_void, ()> {
    let ptr = unsafe {
        libc::mmap(
            base.cast(),
            size,
            PROT_READ | PROT_WRITE,
            MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED,
            -1,
            0,
        )
    };
    if ptr == MAP_FAILED {
        return Err(());
    };

    Ok(ptr)
}

impl PageAllocator for YerbaPageAllocator {
    fn new(page_size: usize) -> Self {
        unsafe {
            // Create the underlying block for storing pointers to arrays of blocks and the sizes
            // of those arrays
            // This will be of type `*mut [PageArray]`
            let page_ptr_ptr = map_arbitrary(size_of::<PageArray>() * 12)
                .expect("Failed to reserve the underlying pointer array");
            let underlying_ptr_array =
                slice_from_raw_parts_mut(page_ptr_ptr as *mut u8, page_size) as *mut [PageArray];

            // The first pointer to the array of page pointers (PageArray)
            // This will be of type `*mut Page`
            let base_page_ptr =
                map_arbitrary(page_size * 12).expect("Failed to reserve initial page array");

            let initial_page_array = PageArray {
                // This is to establish provenance on our `UnsafeCell<[u8]>`
                // In the future we might not do it this way
                pages: slice_from_raw_parts_mut(base_page_ptr, page_size) as *mut Page,
                pages_allocated: AtomicUsize::from(1),
                pages_loaned: AtomicUsize::from(0),
            };
            underlying_ptr_array
                .cast::<PageArray>()
                .write(initial_page_array);

            let first_block_ptr =
                map_fixed(base_page_ptr, page_size).expect("Failed to allocate first page");
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
        if curr_page_array.page_capacity() > curr_page_array.pages_loaned() {
            return curr_page_array
                .pages
                .wrapping_byte_add(size_of::<PageArray>() * curr_page_array.pages_loaned())
                .cast();
        }
        println!("last page addr {:?}", last_page_addr);
        println!("page_count {:?}", curr_page_array.page_capacity());

        match map_fixed(last_page_addr, self.page_size) {
            Ok(page_ptr) => unsafe {
                (*curr_page_array_ptr)
                    .pages_allocated
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                page_ptr.cast()
            },

            Err(_) => {
                println!("failed to allocate page");
                let new_base_page = match map_arbitrary(self.page_size) {
                    Ok(ptr) => ptr,
                    Err(_) => return ptr::null_mut(),
                };
                // `new_base_page` seems to be a lower value than `last_page_addr`
                // which is bad I think
                let new_page_array = PageArray {
                    pages: self.to_page_ptr(new_base_page),
                    pages_allocated: AtomicUsize::from(1),
                    pages_loaned: AtomicUsize::from(0),
                };
                let page_ptr = new_page_array.pages.cast();

                unsafe {
                    // Write the address of the new base page array to the page array buffer
                    // Remeber that this extra layer of indirection is necessary for keeping pages
                    // contiguous whenever possible
                    self.page_block_ptr_array
                        .cast::<PageArray>()
                        .add(page_array_count)
                        .write(new_page_array)
                };

                page_ptr
            }
        }
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
        assert!(!ptr.is_null());

        // TODO Figure out how to effectively clear the pointer to this underlying data in our
        // pointer array
        // Iterate over the page arrays and find which one holds the deallocated pointer
        for i in 0..self.page_array_count() {
            let page_array = unsafe {
                self.page_block_ptr_array
                    .cast::<PageArray>()
                    .wrapping_add(i)
                    .read()
            };
            let page_count = page_array.page_capacity();
            let lower = page_array.pages.addr();
            let upper = lower + page_count * self.page_size;
            if ptr.addr() > lower && ptr.addr() < upper {
                // This is an atomic number so we can edit it through
                // our stack version of page_array in stack memory
                // even though we couldn't edit values of page_array on the heap
                // using it
                page_array.decrement_loaned_page_count();
                unsafe {
                    ptr.write_bytes(0, self.page_size);
                }
            }
        }
    }
}

impl Default for YerbaPageAllocator {
    fn default() -> Self {
        Self::new(DEFAULT_PAGE_SIZE)
    }
}

impl Drop for YerbaPageAllocator {
    fn drop(&mut self) {
        let page_array_count = self.page_array_count();
        let page_blocks = self.page_block_ptr_array.cast::<PageArray>();
        for i in 0..page_array_count {
            let page_array = unsafe { page_blocks.wrapping_add(i).read() };
            unsafe {
                libc::munmap(
                    page_array.pages.cast(),
                    self.page_size * page_array.page_capacity(),
                )
            };
        }
        unsafe {
            libc::munmap(
                self.page_block_ptr_array.cast(),
                size_of::<PageArray>() * 12,
            )
        };
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
