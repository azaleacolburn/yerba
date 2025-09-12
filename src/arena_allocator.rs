use core::{cell::UnsafeCell, sync::atomic::AtomicU8};

const ARENA_SIZE: usize = 4096;
/// Represents a memory block
struct Block {
    size: usize,
    used: bool,
    block: *const Block,
    data: *mut u8,
}

struct ArenaAllocator {
    arena: UnsafeCell<[u8; ARENA_SIZE]>,
    remaining: AtomicU8,
}
