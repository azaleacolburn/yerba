/// Represents a memory block
struct Block {
    size: usize,
    used: bool,
    block: *const Block,
    data: *mut u8,
}

struct Allocator
