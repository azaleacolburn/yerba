use criterion::{BatchSize, Criterion};
use std::hint::black_box;

pub fn c_malloc_free(c: &mut Criterion) {
    c.bench_function("c_malloc_free", |b| {
        b.iter(|| unsafe {
            let t = libc::malloc(5000);
            libc::free(black_box(t));
        });
    });
}

pub fn c_free(c: &mut Criterion) {
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

pub fn box_alloc_free(c: &mut Criterion) {
    c.bench_function("rust_box_alloc_free", |b| {
        b.iter(|| {
            let b: Box<[u8; 5000]> = Box::new([0; 5000]);
            black_box(b);
        });
    });
}
