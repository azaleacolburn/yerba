use core::{alloc::AllocError, ptr::NonNull};

pub const MIN_BLOCK_SIZE: usize = 8;
pub const MAX_BLOCK_SIZE: usize = 4096 * 12; // 24 KB
pub const MAX_ALIGN: usize = 32;
pub const MIN_ALIGN: usize = 1;

pub fn to_non_null_slice<T>(data_ptr: *mut T, size: usize) -> Result<NonNull<[T]>, AllocError> {
    let not_null = NonNull::new(data_ptr).ok_or(AllocError)?;
    Ok(NonNull::<[T]>::slice_from_raw_parts(not_null, size))
}
