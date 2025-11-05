#![feature(allocator_api)]
#![feature(slice_ptr_get)]
// #![no_std]
#![allow(static_mut_refs)]

pub mod array_page_allocator;
pub mod contiguous_header;
pub mod fallback_allocator;
pub mod inline_allocator;
pub mod inline_header;
pub mod page_allocator;
pub mod stack_allocator;
mod util;
pub mod with_page_size;
