#![feature(allocator_api)]
use core::alloc::{Allocator, GlobalAlloc, Layout};
use core::ptr::NonNull;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use yerba::{
    array_page_allocator::ArrayPageAllocator, contiguous_list_allocator::ContiguousListAllocator,
    stack_allocator::StackAllocator,
};

pub fn contiguous_list_create(c: &mut Criterion) {
    c.bench_function("contiguous_list_create", |b| {
        b.iter(|| unsafe {
            let allocator: ContiguousListAllocator<'_, ArrayPageAllocator> =
                ContiguousListAllocator::new();
            black_box(&allocator);
        });
    });
}

pub fn contiguous_list_alloc(c: &mut Criterion) {
    let allocator = ContiguousListAllocator::new();
    let layout = Layout::new::<[u8; 5000]>();
    c.bench_function("contiguous_list_alloc", |b| {
        b.iter(|| unsafe {
            let chunk: NonNull<u8> = allocator.allocate(layout).unwrap().cast();
            black_box(&chunk);
        });
    });
}

fn contiguous_list_free(c: &mut Criterion) {
    let allocator = ContiguousListAllocator::new();
    let layout = Layout::new::<[u8; 5000]>();
    c.bench_function("contiguous_list_free", |b| {
        b.iter_batched(
            || unsafe { allocator.allocate(layout).unwrap().cast() },
            |chunk: NonNull<u8>| unsafe {
                black_box(chunk);
                allocator.deallocate(chunk.cast(), layout);
            },
            BatchSize::SmallInput,
        );
    });
}

fn contiguous_list_alloc_free(c: &mut Criterion) {
    let allocator = ContiguousListAllocator::new();
    let layout = Layout::new::<[u8; 5000]>();
    c.bench_function("contiguous_list_alloc_free", |b| {
        b.iter(|| unsafe {
            let chunk: NonNull<u8> = allocator.allocate(layout).unwrap().cast();
            black_box(chunk);
            allocator.deallocate(chunk.cast(), layout);
        });
    });
}

pub fn stack_create(c: &mut Criterion) {
    c.bench_function("stack_create", |b| {
        b.iter(|| unsafe {
            let allocator = StackAllocator::new();
            black_box(&allocator);
        });
    });
}

fn stack_alloc(c: &mut Criterion) {
    let allocator = StackAllocator::new();
    let layout = Layout::new::<[u8; 5000]>();

    c.bench_function("stack_alloc_free", |b| {
        b.iter(|| unsafe {
            let t: NonNull<u8> = allocator.allocate(layout).unwrap().cast();
            black_box(t);
            allocator.deallocate(t.cast(), layout);
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

fn rust_box_alloc_free(c: &mut Criterion) {
    c.bench_function("rust_box_alloc_free", |b| {
        b.iter(|| {
            let b: Box<[u8; 5000]> = Box::new([0; 5000]);
            black_box(b);
        });
    });
}

fn contiguous_allocator_box_alloc_free(c: &mut Criterion) {
    c.bench_function("contiguous_allocator_box_alloc_free", |b| {
        let allocator = ContiguousListAllocator::new();
        let layout = Layout::new::<[u8; 5000]>();
        b.iter(|| {
            let mut b = Box::<[u8; 5000], &ContiguousListAllocator>::new_in([0; 5000], &allocator);
            black_box(&b);
        });
    });
}

criterion_group!(
    benches,
    contiguous_list_create,
    contiguous_list_alloc,
    contiguous_list_free,
    contiguous_list_alloc_free,
    stack_create,
    stack_alloc,
    c_malloc_free,
    c_free,
    rust_box_alloc_free,
    contiguous_allocator_box_alloc_free
);
criterion_main!(benches);
