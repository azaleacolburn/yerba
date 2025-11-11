#![feature(allocator_api)]
#![feature(slice_ptr_get)]
#![no_std]
#![allow(static_mut_refs)]

#[cfg(test)]
extern crate std;

pub mod array_page_allocator;
pub mod contiguous_header;
pub mod fallback_allocator;
pub mod inline_header;
pub mod linked_header;
pub mod list_allocator;
pub mod mapped_allocator;
pub mod page_allocator;
pub mod stack_allocator;
mod util;
pub mod with_page_size;
