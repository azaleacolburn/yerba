#![feature(allocator_api)]
#![feature(slice_ptr_get)]
#![no_std]
#![allow(static_mut_refs)]

pub mod arena_allocator;
pub mod linear_allocator;
pub mod page_allocator;
