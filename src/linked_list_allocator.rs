use core::{alloc::GlobalAlloc, cell::UnsafeCell, char::MAX, ptr};

const BUF_SIZE: usize = 4096;
const MAX_ALIGN: usize = 32;

/// Represents a memory block
struct Block {
    size: usize,
    used: bool,
    next: *mut Block,
    data: *mut u8,
}

/// head is initially set to buf
struct LinkedListAllocator {
    buf: UnsafeCell<[u8; BUF_SIZE]>,
    head: Block,
}

impl LinkedListAllocator {
    pub fn new() -> Self {
        let buf = UnsafeCell::new([0; BUF_SIZE]);
        Self {
            head: Block {
                size: BUF_SIZE,
                used: false,
                next: ptr::null_mut(),
                data: buf.get() as *mut u8,
            },
            buf,
        }
    }

    fn find_empty_block(&self, size: usize, align: usize) -> *mut Block {
        let mut block_ptr = &self.head as *const Block as *mut Block;
        if align > MAX_ALIGN {
            return ptr::null_mut();
        }
        while !block_ptr.is_null() {
            let block = unsafe { block_ptr.read() };
            if block.used {
                continue;
            }
            let alignment_offset = block.data.align_offset(align);
            if alignment_offset == usize::MAX {
                return ptr::null_mut();
            }
            if block.size >= size + alignment_offset {
                // TODO Verify that this work
                unsafe {
                    (*block_ptr).data = block.data.add(alignment_offset);
                }
                break;
            }
            block_ptr = block.next;
        }

        block_ptr
    }

    /// Finds the block representing the given pointer
    fn find_ptr_block(&self, ptr: *const u8) -> *mut Block {
        let mut block = &self.head as *const Block as *mut Block;
        unsafe {
            while !(*block).data.addr() == ptr.addr() && !block.is_null() {
                block = (*block).next;
            }
        }

        block
    }
}

unsafe impl GlobalAlloc for LinkedListAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();

        let block = self.find_empty_block(size, align);
        if block.is_null() {
            return ptr::null_mut();
        }

        let ptr = unsafe { block.read() }.data;

        if ptr.addr() + size >= unsafe { self.buf.get().byte_add(BUF_SIZE).addr() } {
            return ptr::null_mut();
        }

        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        let block = self.find_ptr_block(ptr);

        unsafe {
            (*block).used = false;
        }
    }

    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
    }

    unsafe fn alloc_zeroed(&self, layout: core::alloc::Layout) -> *mut u8 {}
}
