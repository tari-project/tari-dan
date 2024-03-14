//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

/// Copies a fixed number of bytes from a slice into a fixed-size array and returns T which must define an infallible
/// conversion from the array of the same size.
///
/// # Panics
/// If the slice is not the expected size, a panic will occur. It is therefore up to the caller to ensure that this is
/// the case.
pub fn copy_fixed<const SZ: usize, T>(bytes: &[u8]) -> T
where [u8; SZ]: Into<T> {
    if bytes.len() != SZ {
        panic!(
            "INVARIANT VIOLATION: copy_fixed: expected {} bytes, got {}. Output type: {}",
            SZ,
            bytes.len(),
            std::any::type_name::<T>()
        );
    }
    let mut array = [0u8; SZ];
    array.copy_from_slice(bytes);
    array.into()
}
