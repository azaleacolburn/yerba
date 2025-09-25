use core::{
    alloc::GlobalAlloc,
    cell::UnsafeCell,
    ops::Deref,
    ptr::{self},
};

const BUF_SIZE: usize = 4096;
const MIN_BLOCK_SIZE: usize = 8;
const MAX_ALIGN: usize = 32;
const MIN_ALIGN: usize = 2;

/// Represents a memory block
#[derive(Debug, Default, Clone, Copy)]
struct Block {
    size: usize,
    offset: usize,
}

impl Block {
    pub fn new(size: usize, offset: usize) -> Block {
        Block { size, offset }
    }
}

struct BlockPtr(*mut Block);

impl BlockPtr {
    pub fn null() -> BlockPtr {
        BlockPtr(ptr::null_mut())
    }

    pub fn get_offset(&self) -> usize {
        unsafe { (*self.0).offset & (0 as usize) << (size_of::<usize>() * 8 - 1) }
    }

    pub fn set_offset(&mut self, offset: usize) {
        let used: bool = self.used();
        unsafe {
            (*self.0).offset = offset;
        }
        self.set_used(used);
    }

    pub fn used(&self) -> bool {
        // Seems to be a bit faster or the same as bitshifting
        unsafe { (*self.0).offset.reverse_bits() & 1 == 1 }
    }

    fn set_used(&mut self, used: bool) {
        unsafe {
            let k = size_of::<usize>() * 8 - 1;
            (*self.0).offset &= 0 << k;
            (*self.0).offset &= (used as usize) << k;
        }
    }

    pub fn free(&mut self) {
        self.set_used(false)
    }

    pub fn mark_used(&mut self) {
        self.set_used(true)
    }

    pub fn size(&self) -> usize {
        unsafe { (*self.0).size }
    }

    pub fn add_size(&self, size: usize) {
        unsafe { (*self.0).size += size }
    }

    pub fn set_size(&self, size: usize) {
        unsafe { (*self.0).size = size }
    }

    pub fn set(&mut self, ptr: &BlockPtr) {
        self.0 = ptr.0
    }

    fn get_data(&self) -> *mut u8 {
        let offset = self.get_offset();
        unsafe { self.add(1).byte_add(offset).cast::<u8>() as *mut u8 }
    }
}

impl Deref for BlockPtr {
    type Target = *mut Block;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<*mut Block> for BlockPtr {
    fn from(value: *mut Block) -> Self {
        BlockPtr(value)
    }
}

// Headers are inlined to the buffer
// Only allocates a single buffer and returns a null pointer for allocations past that
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
            buf.get().cast::<Block>().write(head);
        }

        Self { buf }
    }

    fn next_block(&self, block_ptr: &BlockPtr) -> BlockPtr {
        if block_ptr.get_offset() + block_ptr.size() + block_ptr.addr()
            > self.buf_ptr().addr() + BUF_SIZE
        {
            return BlockPtr::null();
        }
        unsafe {
            block_ptr
                .byte_add(block_ptr.get_offset() + block_ptr.size())
                .into()
        }
    }

    fn next_empty_block(&self, block_ptr: &BlockPtr) -> BlockPtr {
        if block_ptr.get_offset() + block_ptr.size() + block_ptr.addr()
            > self.buf_ptr().addr() + BUF_SIZE
        {
            return BlockPtr::null();
        }
        unsafe {
            let mut next: BlockPtr = block_ptr
                .byte_add(block_ptr.get_offset() + block_ptr.size())
                .into();
            if next.is_null() {
                return BlockPtr::null();
            }
            if next.used() {
                next.set(&self.next_empty_block(&next));
            }

            next
        }
    }

    fn next_block_place(&self, block_ptr: &BlockPtr, size: usize) -> BlockPtr {
        if block_ptr.get_offset() + size + block_ptr.addr() > self.buf_ptr().addr() + BUF_SIZE {
            return BlockPtr::null();
        }
        unsafe { block_ptr.byte_add(block_ptr.get_offset() + size).into() }
    }

    fn find_empty_block(&self, size: usize, align: usize) -> BlockPtr {
        let mut last_block_ptr = BlockPtr::null();
        let mut block_ptr = self.first_block();
        if align > MAX_ALIGN {
            return BlockPtr::null();
        }

        while !block_ptr.is_null() {
            unsafe {
                if block_ptr.used() {
                    last_block_ptr.set(&block_ptr);
                    let next_block = &self.next_block(&block_ptr);
                    block_ptr.set(next_block);

                    continue;
                }

                // We don't actually use this pointer again, it's just for calculating the offset
                let data_ptr = block_ptr.add(1).cast::<u8>();
                let alignment_offset = data_ptr.align_offset(align);
                if alignment_offset == usize::MAX {
                    return BlockPtr::null();
                }

                let required_size = size + alignment_offset;
                let fits = block_ptr.size() >= required_size;

                // We've found a block that fits
                if fits {
                    block_ptr.set_offset(alignment_offset);

                    break;
                }

                // We've found a pair of free blocks that can be merged to fit
                let mergeable = !last_block_ptr.is_null() && !last_block_ptr.used();
                if mergeable {
                    let merged_size = block_ptr.size() + last_block_ptr.size();
                    let fits_with_merge = merged_size >= required_size;

                    if fits_with_merge {
                        let data_ptr = last_block_ptr.add(1);
                        let alignment_offset = data_ptr.align_offset(align);
                        if alignment_offset == usize::MAX {
                            return BlockPtr::null();
                        }

                        last_block_ptr.set_offset(alignment_offset);
                        last_block_ptr.add_size(
                            block_ptr.size() + block_ptr.get_offset() + size_of::<Block>(),
                        );

                        block_ptr.write_bytes(0, size_of::<Block>());
                        block_ptr = last_block_ptr;

                        break;
                    }
                }
            }

            last_block_ptr.set(&block_ptr);
            let next_block = &self.next_block(&block_ptr);
            block_ptr.set(next_block);
        }

        block_ptr
    }

    fn first_block(&self) -> BlockPtr {
        BlockPtr(self.buf_ptr() as *mut Block)
    }

    fn last_addr(&self) -> usize {
        unsafe { self.buf_ptr().add(BUF_SIZE).addr() }
    }

    fn buf_ptr(&self) -> *mut u8 {
        self.buf.get().cast()
    }

    /// Finds the block representing the given data pointer
    fn find_ptr_block(&self, ptr: *const u8) -> BlockPtr {
        let mut block = self.first_block();
        unsafe {
            while block.add(1).byte_add(block.get_offset()).addr() != ptr.addr() && !block.is_null()
            {
                block.set(&self.next_block(&block));
            }
        }

        block
    }

    fn number_of_blocks(&self) -> usize {
        let mut c = 0;
        let mut head = self.first_block();
        while !head.is_null() {
            c += 1;
            head.set(&self.next_block(&head));
        }

        c
    }
}

unsafe impl GlobalAlloc for LinkedListAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let size = layout.size();
        let align = layout.align();

        let mut block = self.find_empty_block(size, align);
        if block.is_null() {
            return ptr::null_mut();
        }
        let data_ptr = block.get_data();

        let end_of_block = data_ptr.addr() + size;
        let top_of_buf = self.last_addr();
        if end_of_block > top_of_buf {
            return ptr::null_mut();
        }

        block.mark_used();

        // TODO HUH
        let block_next_ptr = self.next_block_place(&block, size);

        if block.size() > size_of::<Block>() + size
            && (block_next_ptr.addr() + size_of::<Block>()) < self.buf_ptr().addr() + BUF_SIZE
        {
            let new_block_size = block.size() - size_of::<Block>() - size;
            block.set_size(size);
            let new_block = Block {
                size: new_block_size,
                offset: 0,
            };

            unsafe {
                block_next_ptr.write(new_block);
            }
        }

        data_ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: core::alloc::Layout) {
        let mut block = self.find_ptr_block(ptr);

        block.free();
        block.set_offset(0);
    }

    // TODO Fix Infinite Loop
    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        // First look forward for adjacent free blocks
        let mut block_ptr = self.find_ptr_block(ptr);
        block_ptr.free();
        let mut frontier = self.next_block(&block_ptr);
        let mut acc_size = block_ptr.size();
        while acc_size < new_size && !frontier.is_null() {
            if frontier.used() {
                break;
            }

            acc_size += frontier.size() + frontier.get_offset() + size_of::<Block>();

            if acc_size >= new_size {
                let alignment_offset = block_ptr.align_offset(layout.align());
                unsafe {
                    block_ptr.set_offset(alignment_offset);
                    return block_ptr.get_data().add(alignment_offset);
                }
            }
            unsafe { frontier.set(&BlockPtr(frontier.add(1))) };
        }
        if acc_size > new_size {
            return ptr;
        }
        // Then start at the first block and check for available adjacent blocks again
        let mut anchor = self.first_block();
        while !anchor.is_null() {
            if anchor.used() {
                anchor.set(&self.next_block(&anchor));
                continue;
            }

            acc_size = anchor.size();
            frontier.set(&anchor);
            while acc_size < new_size && !frontier.is_null() {
                if frontier.used() {
                    anchor.set(&self.next_block(&frontier));
                    assert!(!anchor.is_null());
                    break;
                }

                acc_size += frontier.size() + frontier.get_offset() + size_of::<Block>();

                if acc_size >= new_size {
                    let alignment_offset = block_ptr.align_offset(layout.align());
                    unsafe {
                        block_ptr.set_offset(alignment_offset);
                        return block_ptr.get_data().add(alignment_offset);
                    }
                }
                unsafe { frontier.set(&BlockPtr(frontier.add(1))) };
            }
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
            allocator.dealloc(one, layout);
            allocator.dealloc(two, layout);
        }
    }

    #[test]
    #[should_panic]
    fn overflow() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 5000]>();

        unsafe {
            let one = allocator.alloc(layout);
            assert!(!one.is_null());

            allocator.dealloc(one, layout);
        }
    }

    #[test]
    fn zeroed() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc_zeroed(layout);
            assert!(!one.is_null());

            let two = allocator.alloc_zeroed(layout);
            assert!(!two.is_null());

            let two_sum: u8 = (0..16).into_iter().map(|i| *(two.wrapping_add(i))).sum();
            let one_sum: u8 = (0..16).into_iter().map(|i| *(one.wrapping_add(i))).sum();
            assert_eq!(two_sum, 0);
            assert_eq!(one_sum, 0);

            allocator.dealloc(two, layout);
            allocator.dealloc(one, layout);
        }
    }

    #[test]
    fn realloc() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 16]>();

        unsafe {
            let one = allocator.alloc(layout);
            assert!(!one.is_null());

            let two = allocator.alloc(layout);
            assert!(!two.is_null());

            allocator.realloc(two, layout, 32);
            allocator.dealloc(one, layout);
            allocator.dealloc(two, Layout::new::<[u8; 32]>());
        }
    }

    #[test]
    fn merge() {
        let allocator = LinkedListAllocator::new();
        let layout = Layout::new::<[u8; 2000]>();

        unsafe {
            let one = allocator.alloc(layout);
            assert!(!one.is_null());
            allocator.dealloc(one, layout);

            let layout = Layout::new::<[u8; 3080]>();
            let two = allocator.alloc(layout);
            assert!(!two.is_null());
        }
    }
}
