use core::alloc::{self, AllocError, GlobalAlloc, Layout};
use core::cmp::{self, max};
use core::ffi::{self, c_void};
use core::ptr::{self, NonNull, null, null_mut};
use libc::{self, mmap, munmap, sbrk};

use lazy_static::lazy_static;

lazy_static! {
    pub static ref PAGE_SIZE: usize = page_size();
}

fn page_size() -> usize {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
}

pub struct YerbaAlloc;

#[global_allocator]
pub static ALLOCATOR: YerbaAlloc = YerbaAlloc {};

// static mut START: *mut Block = null_mut();
// static mut TOP: *mut Block = unsafe { START }; // starts as start

unsafe impl GlobalAlloc for YerbaAlloc {
    unsafe fn alloc(&self, layout: alloc::Layout) -> *mut u8 {
        let aligned_layout = match layout.align_to(cmp::max(layout.align(), *PAGE_SIZE)) {
            Ok(layout) => layout.pad_to_align(),
            Err(_) => return ptr::null_mut(),
        };
        unsafe {
            mmap(
                ptr::null_mut(),
                aligned_layout.size(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        }
        .cast::<u8>()
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

    unsafe fn realloc(&self, ptr: *mut u8, old_layout: alloc::Layout, new_size: usize) -> *mut u8 {
        let layout = Layout::from_size_align(new_size, old_layout.align())
            .expect("Layout from alignment and new size failed");

        let new_ptr = unsafe { self.alloc(layout) };
        (0..layout.size()).for_each(|i| unsafe { new_ptr.add(i).write(ptr.add(i).read()) });

        unsafe { munmap(ptr.cast::<c_void>(), old_layout.size()) };

        new_ptr
    }
}
