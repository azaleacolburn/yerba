#![feature(test)]

use core::alloc::GlobalAlloc;
use std::alloc::Layout;
use yerba::{
    linked_list_allocator::LinkedListAllocator, page_allocator::YerbaPageAllocator,
    stack_allocator::StackAllocator,
};
extern crate test;

#[bench]
fn linked_alloc(b: &mut test::Bencher) {
    let allocator = LinkedListAllocator::<'_, YerbaPageAllocator>::new();
    let layout = Layout::new::<[u8; 5000]>();
    b.iter(|| unsafe {
        let chunk = allocator.alloc(layout);
        test::black_box(&chunk);
    });
}

#[bench]
fn linked_alloc_free(b: &mut test::Bencher) {
    let allocator = LinkedListAllocator::<'_, YerbaPageAllocator>::new();
    let layout = Layout::new::<[u8; 5000]>();
    b.iter(|| unsafe {
        let chunk = unsafe { allocator.alloc(layout) };
        test::black_box(chunk);
        allocator.dealloc(chunk, layout);
    });
}

#[bench]
fn stack_alloc(b: &mut test::Bencher) {
    let allocator = StackAllocator::new();
    let layout = Layout::new::<[u8; 5000]>();

    b.iter(|| unsafe {
        let t = allocator.alloc(layout);
        test::black_box(t);
    })
}

#[bench]
#[unsafe(no_mangle)]
fn c_malloc(b: &mut test::Bencher) {
    b.iter(|| unsafe {
        let t: *mut u8 = libc::calloc(5000, 1).cast();
        test::black_box(&t);
    });
}

#[bench]
fn c_malloc_free(b: &mut test::Bencher) {
    b.iter(|| unsafe {
        let t = unsafe { libc::calloc(5000, 1) };
        test::black_box(&t);
        libc::free(t);
    });
}
