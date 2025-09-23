use core::{alloc::GlobalAlloc, cell::UnsafeCell, ptr};

const BUF_SIZE: usize = 4096;
const MIN_BLOCK_SIZE: usize = 8;
const MAX_ALIGN: usize = 32;
const MIN_ALIGN: usize = 2;

/// Represents a memory block
#[derive(Default, Clone, Copy)]
struct Block {
    size: usize,
    offset: usize,
}

impl Block {
    pub fn new(size: usize, offset: usize) -> Block {
        Block { size, offset }
    }
    pub fn offset(&mut self, offset: usize) {
        let used: bool = self.used();
        self.offset = offset;
        self.set_used(used);
    }

    pub fn used(&self) -> bool {
        // TODO Might be faster to just shift
        self.offset.reverse_bits() & 1 == 1
    }

    pub fn set_used(&mut self, used: bool) {
        self.offset &= (used as usize) << (size_of::<usize>() * 8 - 1);
    }

    pub fn free(&mut self) {
        self.set_used(false)
    }

    pub fn mark_used(&mut self) {
        self.set_used(true)
    }
}

fn get_data(block_ptr: *const Block) -> *mut u8 {
    let offset = unsafe { block_ptr.read().offset };
    unsafe { block_ptr.add(1).byte_add(offset).cast::<u8>() as *mut u8 }
}

/// head is initially set to buf
struct LinkedListAllocator {
    buf: UnsafeCell<[u8; BUF_SIZE]>,
}

fn as_u8_slice<T: Sized>(p: &T) -> &[u8] {
    unsafe { core::slice::from_raw_parts((p as *const T) as *const u8, core::mem::size_of::<T>()) }
}

impl LinkedListAllocator {
    pub fn new() -> Self {
        let buf = UnsafeCell::new([0; BUF_SIZE]);
        let head = Block {
            size: BUF_SIZE,
            offset: 0,
        };

        const {
            let block_size = size_of::<Block>();
            assert!(block_size < BUF_SIZE);
            assert!(block_size % 8 == 0)
        }
        unsafe {
            let block = as_u8_slice(&head);
            buf.get().cast::<&[u8]>().write(block);
        }

        Self { buf }
    }

    fn next_block(&self, block_ptr: *mut Block) -> *mut Block {
        let block = unsafe { *(block_ptr) };
        if block.offset + block.size + block_ptr.addr() > self.buf_ptr().addr() + BUF_SIZE {
            return ptr::null_mut();
        }
        unsafe { block_ptr.byte_add(block.offset + block.size) }
    }

    fn find_empty_block(&self, size: usize, align: usize) -> *mut Block {
        let mut last_block_ptr: *mut Block = ptr::null_mut();
        let mut block_ptr = self.first_block();
        if align > MAX_ALIGN {
            return ptr::null_mut();
        }

        while !block_ptr.is_null() {
            let block = unsafe { block_ptr.read() };
            if block.used() {
                continue;
            }
            let data_ptr = unsafe { block_ptr.add(1).cast::<u8>() };
            let alignment_offset = data_ptr.align_offset(align);
            if alignment_offset == usize::MAX {
                return ptr::null_mut();
            }
            if block.size >= size + alignment_offset + size_of::<Block>() {
                // TODO Verify that this work
                // We've found a block that fits
                unsafe {
                    (*block_ptr).offset = alignment_offset;
                }
                break;
            } else if unsafe {
                !last_block_ptr.is_null()
                    && block.size + (*last_block_ptr).size
                        >= size + alignment_offset + size_of::<Block>()
                    && !(*last_block_ptr).used()
            } {
                // We've found a pair of free blocks that can be merged to fit
                unsafe {
                    (*last_block_ptr).offset = alignment_offset;
                    (*last_block_ptr).size += block.size + size_of::<Block>();

                    block_ptr.write_bytes(0, size_of::<Block>());
                    block_ptr = last_block_ptr;
                }
                break;
            }

            last_block_ptr = block_ptr;
            block_ptr = self.next_block(block_ptr);
        }

        block_ptr
    }

    fn first_block(&self) -> *mut Block {
        self.buf_ptr() as *mut Block
    }

    fn buf_ptr(&self) -> *mut u8 {
        self.buf.get().cast()
    }

    /// Finds the block representing the given data pointer
    fn find_ptr_block(&self, ptr: *const u8) -> *mut Block {
        let mut block = self.first_block();
        unsafe {
            while !block.add((*block).offset).addr() == ptr.addr() && !block.is_null() {
                block = self.next_block(block);
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

        let data_ptr = get_data(block);

        let end_of_block = data_ptr.addr() + size;
        let top_of_buf = unsafe { self.buf.get().byte_add(BUF_SIZE).addr() };
        if end_of_block >= top_of_buf {
            return ptr::null_mut();
        }

        let (block_size, block_next_ptr) = unsafe {
            (*block).mark_used();

            ((*block).size - size, self.next_block(block))
        };

        if block_size > size
            && (self.buf_ptr().addr() + block_next_ptr.addr()) + size_of::<Block>() < BUF_SIZE
        {
            let new_block = Block {
                size: block_size - size,
                offset: 0,
            };

            unsafe {
                block_next_ptr.write(new_block);
            }
        }

        data_ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: core::alloc::Layout) {
        let block = self.find_ptr_block(ptr);

        unsafe {
            (*block).free();
            (*block).offset = 0;
        }
    }

    // TODO Fix Infinite Loop
    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        // First look forward for adjacent free blocks
        let block_ptr = self.find_ptr_block(ptr);
        let mut frontier_ptr = self.next_block(block_ptr);
        let mut acc_size = 0;
        while acc_size < new_size && !frontier_ptr.is_null() {
            let frontier = unsafe { *frontier_ptr };
            if frontier.used() {
                break;
            }

            acc_size += frontier.size + frontier.offset + size_of::<Block>();

            if acc_size >= new_size {
                let alignment_offset = block_ptr.align_offset(layout.align());
                unsafe {
                    (*block_ptr).offset = alignment_offset;
                    return get_data(block_ptr).add(alignment_offset);
                }
            }
            frontier_ptr = unsafe { frontier_ptr.add(1) };
        }
        // Then start at the first block and check for available adjacent blocks again
        let mut head = self.first_block();
        while !head.is_null() {
            acc_size = 0;
            frontier_ptr = head;
            while acc_size < new_size && !frontier_ptr.is_null() {
                let frontier = unsafe { *frontier_ptr };
                if frontier.used() {
                    break;
                }

                acc_size += frontier.size + frontier.offset + size_of::<Block>();

                if acc_size >= new_size {
                    let alignment_offset = block_ptr.align_offset(layout.align());
                    unsafe {
                        (*block_ptr).offset = alignment_offset;
                        return get_data(block_ptr).add(alignment_offset);
                    }
                }
                frontier_ptr = unsafe { frontier_ptr.add(1) };
            }

            head = frontier_ptr;
        }

        // TODO Request page
        ptr::null_mut()
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
            allocator.dealloc(two, layout);
            allocator.dealloc(one, layout);
        }
    }

    // #[test]
    // #[should_panic]
    // fn out_of_order() {
    //     let allocator = LinkedListAllocator::new();
    //     let layout = Layout::new::<[u8; 16]>();
    //
    //     unsafe {
    //         let one = allocator.alloc(layout);
    //         assert!(!one.is_null());
    //
    //         let two = allocator.alloc(layout);
    //         assert!(!two.is_null());
    //
    //         allocator.dealloc(one, layout);
    //     }
    // }

    #[test]
    fn zeroed() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc_zeroed(layout);
            assert!(!one.is_null());

            let two = allocator.alloc_zeroed(layout);
            assert!(!two.is_null());

            allocator.dealloc(two, layout);
            allocator.dealloc(one, layout);
        }
    }

    #[test]
    fn realloc() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc_zeroed(layout);
            assert!(!one.is_null());

            let two = allocator.alloc_zeroed(layout);
            assert!(!two.is_null());

            allocator.realloc(two, layout, 32);
            allocator.dealloc(two, Layout::new::<[u8; 32]>());
            allocator.dealloc(one, layout);
        }
    }
}
