use std::{alloc::AllocError, ptr::NonNull};

pub fn to_non_null_slice<T>(data_ptr: *mut T, size: usize) -> Result<NonNull<[T]>, AllocError> {
    let not_null = NonNull::new(data_ptr).ok_or_else(|| AllocError)?;
    Ok(NonNull::<[T]>::slice_from_raw_parts(not_null, size))
}
