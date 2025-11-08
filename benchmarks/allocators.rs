#![feature(allocator_api)]
use core::alloc::{Allocator, GlobalAlloc, Layout};
use core::ptr::NonNull;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use yerba::{
    array_page_allocator::ArrayPageAllocator, list_allocator::ListAllocator,
    stack_allocator::StackAllocator,
};

pub fn contiguous_list_create(c: &mut Criterion) {
    c.bench_function("contiguous_list_create", |b| {
        b.iter(|| unsafe {
            let allocator: ListAllocator<'_, ArrayPageAllocator> = ListAllocator::default();
            black_box(&allocator);
        });
    });
}

pub fn contiguous_list_alloc(c: &mut Criterion) {
    let allocator: ListAllocator<'_, ArrayPageAllocator> = ListAllocator::default();
    let layout = Layout::new::<[u8; 5000]>();
    // Ideally this works and all the memory gets freed
    c.bench_function("contiguous_list_alloc", |b| {
        b.iter(|| unsafe {
            let chunk: NonNull<u8> = allocator.allocate(layout).unwrap().cast();
            black_box(&chunk);
        });
    });
}

fn contiguous_list_free(c: &mut Criterion) {
    let allocator: ListAllocator<'_, ArrayPageAllocator> = ListAllocator::default();
    let layout = Layout::new::<[u8; 5000]>();
    c.bench_function("contiguous_list_free", |b| {
        b.iter_batched(
            || unsafe { allocator.allocate(layout).unwrap().cast() },
            |chunk: NonNull<u8>| unsafe {
                allocator.deallocate(black_box(chunk.cast()), layout);
            },
            BatchSize::SmallInput,
        );
    });
}

fn contiguous_list_alloc_free(c: &mut Criterion) {
    let allocator: ListAllocator<'_, ArrayPageAllocator> = ListAllocator::default();
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
            let allocator = StackAllocator::default();
            black_box(&allocator);
        });
    });
}

fn stack_alloc(c: &mut Criterion) {
    let allocator = StackAllocator::default();
    let layout = Layout::new::<[u8; 5000]>();

    c.bench_function("stack_alloc_free", |b| {
        b.iter(|| unsafe {
            let t: NonNull<u8> = allocator.allocate(layout).unwrap().cast();
            black_box(t);
            allocator.deallocate(t.cast(), layout);
        })
    });
}

fn stack_free(c: &mut Criterion) {
    let allocator = StackAllocator::default();
    let layout = Layout::new::<[u8; 5000]>();

    c.bench_function("stack_free", |b| {
        b.iter_batched(
            || unsafe {
                let t: NonNull<u8> = black_box(allocator.allocate(layout).unwrap().cast());
                t
            },
            |t| unsafe {
                allocator.deallocate(t.cast(), layout);
            },
            BatchSize::PerIteration,
        )
    });
}

fn c_malloc_free(c: &mut Criterion) {
    c.bench_function("c_malloc_free", |b| {
        b.iter(|| unsafe {
            let t = libc::malloc(5000);
            libc::free(black_box(t));
        });
    });
}

fn c_free(c: &mut Criterion) {
    c.bench_function("c_free", |b| {
        b.iter_batched(
            || unsafe { black_box(libc::malloc(5000)) },
            |chunk| unsafe {
                libc::free(chunk);
            },
            BatchSize::PerIteration,
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
        let allocator = ListAllocator::default();
        let layout = Layout::new::<[u8; 5000]>();
        b.iter(|| {
            let mut b = Box::<[u8; 5000], &ListAllocator>::new_in([0; 5000], &allocator);
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
    stack_free,
    stack_alloc,
    c_malloc_free,
    c_free,
    rust_box_alloc_free,
    contiguous_allocator_box_alloc_free
);
criterion_main!(benches);
