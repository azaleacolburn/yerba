use core::alloc::GlobalAlloc;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use std::alloc::Layout;
use yerba::{
    linked_list_allocator::LinkedListAllocator, page_allocator::YerbaPageAllocator,
    stack_allocator::StackAllocator,
};

pub fn linked_alloc(c: &mut Criterion) {
    let allocator = LinkedListAllocator::<'_, YerbaPageAllocator>::new();
    let layout = Layout::new::<[u8; 5000]>();
    c.bench_function("linked_alloc", |b| {
        b.iter(|| unsafe {
            let chunk = allocator.alloc(layout);
            black_box(&chunk);
        });
    });
}

fn linked_alloc_free(c: &mut Criterion) {
    let allocator = LinkedListAllocator::<'_, YerbaPageAllocator>::new();
    let layout = Layout::new::<[u8; 5000]>();
    c.bench_function("linked_alloc_free", |b| {
        b.iter(|| unsafe {
            let chunk = unsafe { allocator.alloc(layout) };
            black_box(chunk);
            allocator.dealloc(chunk, layout);
        });
    });
}

fn stack_alloc(c: &mut Criterion) {
    let allocator = StackAllocator::new();
    let layout = Layout::new::<[u8; 5000]>();

    c.bench_function("stack_alloc", |b| {
        b.iter(|| unsafe {
            let t = allocator.alloc(layout);
            black_box(t);
        })
    });
}

fn c_malloc_free(c: &mut Criterion) {
    c.bench_function("c_malloc", |b| {
        b.iter(|| unsafe {
            let t: *mut u8 = libc::calloc(5000, 1).cast();
            black_box(&t);
            libc::free(t.cast());
        });
    });
}

fn c_free(c: &mut Criterion) {
    c.bench_function("c_free", |b| {
        b.iter_batched(
            || unsafe { libc::calloc(5000, 1) },
            |chunk| unsafe {
                libc::free(black_box(chunk));
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    benches,
    linked_alloc,
    linked_alloc_free,
    stack_alloc,
    c_malloc_free,
    c_free,
);
criterion_main!(benches);
