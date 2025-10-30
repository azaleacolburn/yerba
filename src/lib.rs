#![feature(allocator_api)]
#![feature(slice_ptr_get)]
// #![no_std]
#![allow(static_mut_refs)]

// pub mod linear_allocator;
pub mod contiguous_list_allocator;
pub mod fallback_allocator;
mod page_allocator;
pub mod page_allocator_trait;
pub mod stack_allocator;
mod util;
