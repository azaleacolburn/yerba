#![feature(allocator_api)]
#![feature(slice_ptr_get)]
// #![no_std]
#![allow(static_mut_refs)]

pub mod array_page_allocator;
pub mod contiguous_list_allocator;
pub mod fallback_allocator;
pub mod newable;
pub mod page_allocator;
pub mod stack_allocator;
mod util;
