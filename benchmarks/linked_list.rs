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
        // chunk.write_bytes(10, 5000);
    });
}

#[bench]
fn linked_dealloc(b: &mut test::Bencher) {
    let allocator = LinkedListAllocator::<'_, YerbaPageAllocator>::new();
    let layout = Layout::new::<[u8; 5000]>();
    let chunk = unsafe { allocator.alloc(layout) };
    assert!(!chunk.is_null());
    b.iter(|| unsafe {
        allocator.dealloc(chunk, layout);
    });
}

#[bench]
fn stack_alloc(b: &mut test::Bencher) {
    let allocator = StackAllocator::new();
    let layout = Layout::new::<[u8; 5000]>();

    b.iter(|| unsafe {
        let t = allocator.alloc(layout);
    })
}

#[bench]
fn c_malloc(b: &mut test::Bencher) {
    b.iter(|| unsafe {
        for i in 0..100 {
            let t: *mut u8 = libc::calloc(5000, 1).cast();
            t.write_bytes(10, 5000);
            for j in 0..5000 {
                println!("{}", t.wrapping_add(j).read());
            }
        }
    });
}

#[bench]
fn c_free(b: &mut test::Bencher) {
    let t = unsafe { libc::calloc(5000, 1) };
    b.iter(|| unsafe {
        libc::free(t);
    });
}
