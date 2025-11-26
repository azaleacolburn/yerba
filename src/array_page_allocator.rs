use crate::page_allocator::PageAllocator;
use crate::with_page_size::WithPageSize;
use core::cell::UnsafeCell;
use core::ffi::{c_int, c_void};
use core::ptr::slice_from_raw_parts_mut;
use core::ptr::{self};
use libc::{
    self, MAP_ANONYMOUS, MAP_FAILED, MAP_FIXED_NOREPLACE, MAP_SHARED, MREMAP_FIXED, PROT_READ,
    PROT_WRITE, mremap,
};

const DEFAULT_PAGE_SIZE: usize = 4096;

type Page = UnsafeCell<[u8]>;

/// An allocator for managing entire pages of memory
/// Intended to be used by higher level allocators to abstact away
/// memory requests to the operating system
///
/// The pages are not guaranteed to be contiguous to each other, nor are they guaranteed
/// Unless a page larger than the default size, in which case it will be allocated
///
/// Pages are not guaranteed to be contiguous with respect to each other.
/// The allocator will attempt to make them contiguous, however, if the allocation of a contiguous
/// block with mmap(fixed) fails, a new base of contiguous pages will be allocated.
pub struct ArrayPageAllocator<'a> {
    page_size: usize,
    // Represents the number of page_arrays we've allocated
    page_array_count: u8,
    page_array_buffer: &'a mut [PageArray],
}

#[derive(Debug)]
struct PageArray {
    // Secretly, this is actually an array of pages
    // This is to avoid using an extra usize of space, since we functionally need both
    // the capacity (which is fixed the `PageAllocator::page_size`) and the length (which is the variable `page_count`)
    //
    // If `pages` where of type `*mut [Page]`, we would also have to store the page_size as the
    // wide pointer's provenance, which we're already storing in `PageAllocator`, our parent struct
    pages: *mut Page,
    pages_loaned: usize,
    pages_allocated: usize,
}

impl PageArray {
    fn last_addr(&self, page_size: usize) -> *mut c_void {
        unsafe {
            let first_page_ptr = self.pages.cast::<*mut Page>();
            first_page_ptr
                .byte_add(page_size * self.pages_allocated - 1)
                .cast()
        }
    }

    fn _set_loaned_page_count(&mut self, n: impl Into<usize>) {
        self.pages_loaned = n.into();
    }

    fn decrement_loaned_page_count(&mut self) {
        self.pages_loaned -= 1;
    }

    fn increment_loaned_page_count(&mut self) {
        self.pages_loaned += 1;
    }

    fn _set_allocated_page_count(&mut self, n: impl Into<usize>) {
        self.pages_allocated = n.into();
    }

    fn decrement_allocated_page_count(&mut self) {
        self.pages_allocated -= 1;
    }

    fn increment_allocated_page_count(&mut self) {
        self.pages_allocated += 1;
    }
}

impl<'a> WithPageSize for ArrayPageAllocator<'a> {
    fn with_page_size(page_size: usize) -> Self {
        unsafe {
            // Create the underlying block for storing pointers to arrays of blocks and the sizes
            // of those arrays
            // This will be of type `*mut [PageArray]`
            let page_ptr_ptr = map_arbitrary(size_of::<PageArray>() * 12)
                .expect("Failed to reserve the underlying pointer array");
            let underlying_ptr_array =
                &mut *(slice_from_raw_parts_mut(page_ptr_ptr as *mut u8, page_size)
                    as *mut [PageArray]);

            // The first pointer to the array of page pointers (PageArray)
            // This will be of type `*mut Page`
            // let base_page_ptr =
            //     map_arbitrary(page_size).expect("Failed to reserve initial page array");

            // let initial_page_array = PageArray {
            //     // This is to establish provenance on our `UnsafeCell<[u8]>`
            //     // In the future we might not do it this way
            //     pages: slice_from_raw_parts_mut(base_page_ptr, page_size) as *mut Page,
            //     pages_allocated: 1,
            //     pages_loaned: 0,
            // };
            // underlying_ptr_array[0] = initial_page_array;

            ArrayPageAllocator {
                page_size,
                page_array_count: 0,
                page_array_buffer: underlying_ptr_array,
            }
        }
    }
}

fn map_generic(base: *mut c_void, size: usize, flags: c_int) -> Result<*mut c_void, ()> {
    let ptr = unsafe { libc::mmap(base, size, PROT_READ | PROT_WRITE, flags, -1, 0) };

    if ptr == MAP_FAILED {
        return Err(());
    };

    Ok(ptr)
}

fn map_arbitrary(size: usize) -> Result<*mut c_void, ()> {
    map_generic(ptr::null_mut(), size, MAP_ANONYMOUS | MAP_SHARED)
}

fn map_fixed<T>(base: *mut T, size: usize) -> Result<*mut c_void, ()> {
    map_generic(
        base.cast(),
        size,
        MAP_ANONYMOUS | MAP_SHARED | MAP_FIXED_NOREPLACE,
    )
}

impl<'a> ArrayPageAllocator<'a> {
    fn current_page_array(&mut self) -> &mut PageArray {
        &mut self.page_array_buffer[self.page_array_count as usize - 1]
    }

    fn find<T>(&mut self, ptr: *mut T) -> Option<&mut PageArray> {
        for i in 0..self.page_array_count as usize {
            let page_array = &self.page_array_buffer[i];

            let buf_ptr: *mut T = page_array.pages.cast();
            let end = unsafe { buf_ptr.byte_add(page_array.pages_loaned * self.page_size) };

            if (buf_ptr..end).contains(&ptr) {
                return Some(&mut self.page_array_buffer[i]);
            }
        }

        None
    }
}

impl<'a> PageAllocator for ArrayPageAllocator<'a> {
    unsafe fn request_page(&mut self) -> *mut u8 {
        let page_array_count = self.page_array_count as usize;
        assert!(page_array_count < 12); // TODO Write code to resize header buffer

        let new_base_page = match map_arbitrary(self.page_size * 4) {
            Ok(ptr) => ptr,
            Err(_) => return ptr::null_mut(),
        };

        let new_page_array = PageArray {
            pages: self.to_page_ptr(new_base_page),
            pages_allocated: 4,
            pages_loaned: 1,
        };

        // Write the address of the new base page array to the page array buffer
        self.page_array_buffer[page_array_count] = new_page_array;
        self.page_array_count += 1;

        new_base_page.cast()
    }

    unsafe fn request_page_zeroed(&mut self) -> *mut u8 {
        unsafe {
            let address = self.request_page();
            if address.is_null() {
                return ptr::null_mut();
            }
            address.write_bytes(0, self.page_size);

            address
        }
    }

    unsafe fn relinquish_page(&mut self, ptr: *mut u8) {
        assert!(!ptr.is_null());

        // TODO Figure out how to effectively clear the pointer to this underlying data in our
        // pointer array
        // Iterate over the page arrays and find which one holds the deallocated pointer
        for i in 0..self.page_array_count as usize {
            let page_array = &mut self.page_array_buffer[i];
            let page_count = page_array.pages_allocated;
            let lower = page_array.pages.addr();
            let upper = lower + page_count * self.page_size;
            if ptr.addr() > lower && ptr.addr() < upper {
                page_array.decrement_allocated_page_count();
                unsafe {
                    ptr.write_bytes(0, self.page_size);
                }
            }
        }
    }

    fn get_pages_allocated(&self) -> usize {
        let page_array_count = self.page_array_count as usize;

        // TODO Make code style choices
        (0..page_array_count)
            .into_iter()
            .map(|i| self.page_array_buffer[i].pages_loaned)
            .sum()
    }

    fn get_page_size(&self) -> usize {
        self.page_size
    }

    unsafe fn extend_page(&mut self, ptr: *mut u8, added_size: usize) -> bool {
        let page_size = self.page_size;
        let page_array = self.find(ptr).unwrap();
        if page_array.pages_allocated > page_array.pages_loaned {
            let diff = (page_array.pages_allocated - page_array.pages_loaned) * page_size;
            if diff >= added_size {
                page_array.increment_loaned_page_count();

                return true;
            }

            page_array.pages_loaned = page_array.pages_allocated;
        }

        let old_size = page_array.pages_loaned * page_size;
        page_array.increment_loaned_page_count();
        let new_size = page_array.pages_loaned * page_size;

        match unsafe { mremap(ptr.cast(), old_size, new_size, MREMAP_FIXED) } {
            MAP_FAILED => {
                println!("Remap Failed");
                page_array.decrement_loaned_page_count();
                false
            }
            new_ptr => {
                assert_eq!(new_ptr.cast(), ptr);
                // page_array.increment_loaned_page_count();
                page_array.increment_allocated_page_count();

                true
            }
        }
    }
}

impl<'a> Default for ArrayPageAllocator<'a> {
    fn default() -> Self {
        Self::with_page_size(DEFAULT_PAGE_SIZE)
    }
}

impl<'a> Drop for ArrayPageAllocator<'a> {
    fn drop(&mut self) {
        let page_blocks = &self.page_array_buffer;
        for i in 0..(self.page_array_count as usize) {
            let page_array = &page_blocks[i];
            unsafe {
                libc::munmap(
                    page_array.pages.cast(),
                    self.page_size * page_array.pages_allocated,
                )
            };
        }

        // I this memory needs to be unmapped, even though
        // it was cast to a slice
        unsafe {
            let success = libc::munmap(
                self.page_array_buffer as *mut [PageArray] as *mut c_void,
                size_of::<PageArray>() * 12,
            );
            if success == -1 {
                panic!("Failed to unmap memory chunk");
            }
        };
    }
}

impl<'a> ArrayPageAllocator<'a> {
    fn to_page_ptr<T: ?Sized>(&self, ptr: *mut T) -> *mut Page {
        slice_from_raw_parts_mut(ptr.cast::<u8>(), self.page_size) as *mut Page
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn alloc_chunks() {
        let mut allocator = ArrayPageAllocator::default();

        unsafe {
            let page = allocator.request_page();
            assert!(!page.is_null());
            allocator.relinquish_page(page);

            let one = allocator.request_page();
            assert!(!one.is_null());

            let two = allocator.request_page();
            assert!(!two.is_null());

            allocator.relinquish_page(one);
            allocator.relinquish_page(two);
        }
    }

    #[test]
    fn overflow() {
        let mut allocator = ArrayPageAllocator::with_page_size(DEFAULT_PAGE_SIZE * 12);

        unsafe {
            let one = allocator.request_page();
            assert!(!one.is_null());
            allocator.relinquish_page(one);

            let two = allocator.request_page();
            assert!(!two.is_null());
            allocator.relinquish_page(two);
        }
    }

    #[test]
    fn zeroed() {
        let mut allocator = ArrayPageAllocator::default();

        unsafe {
            let one = allocator.request_page_zeroed();
            assert!(!one.is_null());

            let two = allocator.request_page_zeroed();
            assert!(!two.is_null());

            let two_sum: u8 = (0..16).into_iter().map(|i| *(two.wrapping_add(i))).sum();
            let one_sum: u8 = (0..16).into_iter().map(|i| *(one.wrapping_add(i))).sum();
            assert_eq!(two_sum, 0);
            assert_eq!(one_sum, 0);

            allocator.relinquish_page(two);
            allocator.relinquish_page(one);
        }
    }
}
