use core::alloc::{self, GlobalAlloc, Layout};
use core::cmp;
use core::ffi::c_void;
use core::ptr;
use libc::{self, mmap, munmap};

use lazy_static::lazy_static;

lazy_static! {
    pub static ref PAGE_SIZE: usize = page_size();
}

fn page_size() -> usize {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
}

/// Represents a memory block
struct Block {
    size: usize,
    used: bool,
    block: *const Block,
    data: *mut u8,
}

pub struct PageAlloc;

// static mut START: *mut Block = null_mut();
// static mut TOP: *mut Block = unsafe { START }; // starts as start

unsafe impl GlobalAlloc for PageAlloc {
    unsafe fn alloc(&self, layout: alloc::Layout) -> *mut u8 {
        let aligned_layout = match layout.align_to(cmp::max(layout.align(), *PAGE_SIZE)) {
            Ok(layout) => layout.pad_to_align(),
            Err(_) => return ptr::null_mut(),
        };
        unsafe {
            mmap(
                ptr::null_mut(),
                aligned_layout.size(),
                // The memory will be read and written to
                libc::PROT_READ | libc::PROT_WRITE,
                // The changes to the memory will not be visible to other processes
                // and this memory is not tied back to any specific file
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                // No file descriptor
                -1,
                // 0 offset
                0,
            )
        }
        .cast::<u8>()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: alloc::Layout) {
        let size = layout.size();
        unsafe { munmap(ptr.cast::<c_void>(), size) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, old_layout: alloc::Layout, new_size: usize) -> *mut u8 {
        let layout = Layout::from_size_align(new_size, old_layout.align())
            .expect("Layout from alignment and new size failed");

        let new_ptr = unsafe { self.alloc(layout) };
        (0..layout.size()).for_each(|i| unsafe { new_ptr.add(i).write(ptr.add(i).read()) });

        unsafe { munmap(ptr.cast::<c_void>(), old_layout.size()) };

        new_ptr
    }
}
