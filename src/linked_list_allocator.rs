use core::{alloc::GlobalAlloc, cell::UnsafeCell, char::MAX, ptr};

use libc::sbrk;

const BUF_SIZE: usize = 4096;
const MAX_ALIGN: usize = 32;
const MIN_ALIGN: usize = 2;

/// Represents a memory block
#[derive(Default, Clone, Copy)]
struct Block {
    used: bool,
    size: usize,
    offset: usize,
}

impl Block {
    fn next_block(&self, self_ptr: *mut Block, buf_ptr: usize) -> *mut Block {
        if self.offset + self.size + self_ptr.addr() > buf_ptr + BUF_SIZE {
            return ptr::null_mut();
        }
        unsafe { self_ptr.byte_add(self.offset + self.size) }
    }
}

fn get_data(block_ptr: *const Block) -> *mut u8 {
    unsafe { block_ptr.add(1).cast::<u8>() as *mut u8 }
}

/// head is initially set to buf
struct LinkedListAllocator {
    buf: UnsafeCell<[u8; BUF_SIZE]>,
    blocks: [Block; BUF_SIZE / MAX_ALIGN],
}

unsafe fn as_u8_slice<T: Sized>(p: &T) -> &[u8] {
    unsafe { core::slice::from_raw_parts((p as *const T) as *const u8, core::mem::size_of::<T>()) }
}

impl LinkedListAllocator {
    pub fn new() -> Self {
        let buf = UnsafeCell::new([0; BUF_SIZE]);
        let head = Block {
            size: BUF_SIZE,
            used: false,
            offset: 0,
        };

        const {
            assert!(size_of::<Block>() < BUF_SIZE);
        }
        unsafe {
            buf.get().write(as_u8_slice(&head).try_into().unwrap());
        }

        Self {
            blocks: [head; BUF_SIZE / MAX_ALIGN],
            buf,
        }
    }

    fn find_empty_block(&self, size: usize, align: usize) -> *mut Block {
        let mut block_ptr = &self.blocks[0] as *const Block as *mut Block;
        if align > MAX_ALIGN {
            return ptr::null_mut();
        }
        while !block_ptr.is_null() {
            let block = unsafe { block_ptr.read() };
            if block.used {
                continue;
            }
            let data_ptr = unsafe { block_ptr.add(1).cast::<u8>() };
            let alignment_offset = data_ptr.align_offset(align);
            if alignment_offset == usize::MAX {
                return ptr::null_mut();
            }
            if block.size >= size + alignment_offset + size_of::<Block>() {
                // TODO Verify that this work
                unsafe {
                    (*block_ptr).offset = alignment_offset;
                }
                break;
            }
            let next = unsafe { block_ptr.add(size + align) };
            block_ptr = next;
        }

        block_ptr
    }

    fn first_block(&self) -> *mut Block {
        &self.blocks[0] as *const Block as *mut Block
    }

    fn buf_ptr(&self) -> *mut u8 {
        self.buf.get().cast()
    }

    /// Finds the block representing the given pointer
    fn find_ptr_block(&self, ptr: *const u8) -> *mut Block {
        let mut block = self.first_block();
        unsafe {
            while !block.add((*block).offset).addr() == ptr.addr() && !block.is_null() {
                block = (*block).next_block(block, self.buf_ptr().addr());
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

        let data_ptr = unsafe { get_data(block) };

        let end_of_block = data_ptr.addr() + size;
        let top_of_buf = unsafe { self.buf.get().byte_add(BUF_SIZE).addr() };
        if end_of_block >= top_of_buf {
            return ptr::null_mut();
        }

        let (block_size, block_next) = unsafe { ((*block).size - size, (*block).next) };
        if block_size > size {
            let mut new_block = Block {
                size: block_size - size,
                data: end_of_block as *mut u8,
                used: false,
                next: block_next,
            };

            unsafe { (*block).next = &mut new_block as *mut Block }
        }

        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        let block = self.find_ptr_block(ptr);

        unsafe {
            (*block).used = false;
            (*block).offset = 0;
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
