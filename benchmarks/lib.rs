#![feature(allocator_api)]
#![allow(clippy::all)]
use criterion::{criterion_group, criterion_main};

pub mod builtin;
pub mod contiguous_list;
pub mod linked_list;
pub mod mapped;
pub mod stack;

criterion_group!(
    benches,
    contiguous_list::create,
    contiguous_list::alloc,
    contiguous_list::free,
    contiguous_list::alloc_free,
    contiguous_list::box_alloc_free,
    linked_list::create,
    linked_list::alloc,
    linked_list::free,
    linked_list::alloc_free,
    linked_list::box_alloc_free,
    mapped::create,
    mapped::alloc,
    mapped::free,
    mapped::alloc_free,
    mapped::box_alloc_free,
    stack::create,
    stack::free,
    stack::alloc_free,
    builtin::c_malloc_free,
    builtin::c_free,
    builtin::box_alloc_free,
);
criterion_main!(benches);
