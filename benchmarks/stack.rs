use core::alloc::{Allocator, Layout};
use core::ptr::NonNull;
use criterion::{BatchSize, Criterion, black_box};
use yerba::stack_allocator::StackAllocator;

pub fn create(c: &mut Criterion) {
    c.bench_function("stack_create", |b| {
        b.iter(|| unsafe {
            let allocator = StackAllocator::default();
            black_box(&allocator);
        });
    });
}

pub fn alloc_free(c: &mut Criterion) {
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

pub fn free(c: &mut Criterion) {
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
