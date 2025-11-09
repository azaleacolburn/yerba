use core::alloc::{Allocator, GlobalAlloc, Layout};
use core::ptr::NonNull;
use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use yerba::linked_header::LinkedHeader;
use yerba::{
    array_page_allocator::ArrayPageAllocator, list_allocator::ListAllocator,
    stack_allocator::StackAllocator,
};

pub fn create(c: &mut Criterion) {
    c.bench_function("linked_list_create", |b| {
        b.iter(|| unsafe {
            let allocator: ListAllocator<'_, ArrayPageAllocator, LinkedHeader> =
                ListAllocator::default();
            black_box(&allocator);
        });
    });
}

pub fn alloc(c: &mut Criterion) {
    let allocator: ListAllocator<'_, ArrayPageAllocator, LinkedHeader> = ListAllocator::default();
    let layout = Layout::new::<[u8; 5000]>();
    // Ideally this works and all the memory gets freed
    c.bench_function("linked_list_alloc", |b| {
        b.iter(|| unsafe {
            let chunk: NonNull<u8> = allocator.allocate(layout).unwrap().cast();
            black_box(&chunk);
        });
    });
}

pub fn free(c: &mut Criterion) {
    let allocator: ListAllocator<'_, ArrayPageAllocator, LinkedHeader> = ListAllocator::default();
    let layout = Layout::new::<[u8; 5000]>();
    c.bench_function("linked_list_free", |b| {
        b.iter_batched(
            || unsafe { allocator.allocate(layout).unwrap().cast() },
            |chunk: NonNull<u8>| unsafe {
                allocator.deallocate(black_box(chunk.cast()), layout);
            },
            BatchSize::SmallInput,
        );
    });
}

pub fn alloc_free(c: &mut Criterion) {
    let allocator: ListAllocator<'_, ArrayPageAllocator, LinkedHeader> = ListAllocator::default();
    let layout = Layout::new::<[u8; 5000]>();
    c.bench_function("linked_list_alloc_free", |b| {
        b.iter(|| unsafe {
            let chunk: NonNull<u8> = allocator.allocate(layout).unwrap().cast();
            black_box(chunk);
            allocator.deallocate(chunk.cast(), layout);
        });
    });
}

pub fn box_alloc_free(c: &mut Criterion) {
    c.bench_function("linked_list_box_alloc_free", |b| {
        let allocator = ListAllocator::<'_, ArrayPageAllocator, LinkedHeader>::default();
        let layout = Layout::new::<[u8; 5000]>();
        b.iter(|| {
            let mut b = Box::<[u8; 5000], &ListAllocator<ArrayPageAllocator, LinkedHeader>>::new_in(
                [0; 5000], &allocator,
            );
            black_box(&b);
        });
    });
}
