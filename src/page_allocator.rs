use core::alloc::{self, AllocError, GlobalAlloc, Layout};
use core::cmp::{self, max};
use core::ffi::{self, c_void};
use core::ptr::{self, NonNull, null, null_mut};
use libc::{
    self, MAP_ANONYMOUS, MAP_FAILED, MAP_FIXED, MAP_NORESERVE, MAP_PRIVATE, MAP_SHARED, PROT_READ,
    PROT_WRITE, mmap, munmap, sbrk,
};
use std::cell::UnsafeCell;
use std::fmt::Pointer;
use std::ptr::slice_from_raw_parts_mut;
use std::sync::atomic::AtomicUsize;

use lazy_static::lazy_static;

lazy_static! {
    pub static ref PAGE_SIZE: usize = page_size();
}

fn page_size() -> usize {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
}

const DEFAULT_PAGE_SIZE: usize = 4096;

/// An allocator for managing entire pages of memory
/// Intended to be used by higher level allocators to abstact away
/// memory requests to the operating system
///
/// The pages are not guaranteed to be contiguous to each other, nor are they guaranteed
/// Unless a page larger than the default size, in which case it will be allocated
///
/// Pages are not guaranteed to be contiguous with respect to each other
/// The allocator will attempt to make them contiguous, however, if the allocation of a contiguous
/// block with mmap(fixed)  
///
///
pub struct PageAllocator {
    page_size: usize,
    page_count: AtomicUsize,
    base: *mut UnsafeCell<[u8]>,
}

impl PageAllocator {
    fn new(page_size: usize) -> Self {
        unsafe {
            let base_addr = libc::mmap(
                ptr::null_mut(),
                page_size * 12,
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
                page_size,
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED,
                -1,
                0,
            );
            if mem_ptr == MAP_FAILED {
                panic!("Failed to allocate first page");
            }
            assert_eq!(mem_ptr, base_addr);

            let base =
                slice_from_raw_parts_mut(mem_ptr as *mut u8, page_size) as *mut UnsafeCell<[u8]>;

            PageAllocator {
                page_size,
                page_count: AtomicUsize::from(1),
                base,
            }
        }
    }

    fn last_addr(&self) -> *mut c_void {
        unsafe {
            self.base
                .byte_add(self.page_size * self.page_count())
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

impl Default for PageAllocator {
    fn default() -> Self {
        Self::new(DEFAULT_PAGE_SIZE)
    }
}

unsafe impl GlobalAlloc for PageAllocator {
    unsafe fn alloc(&self, _layout: alloc::Layout) -> *mut u8 {
        let ptr = unsafe {
            libc::mmap(
                self.last_addr(),
                self.page_size,
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE | MAP_FIXED,
                -1,
                0,
            )
            .cast::<u8>()
        };
        self.page_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        ptr
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
