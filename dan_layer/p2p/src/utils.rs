//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub(crate) fn checked_copy_fixed<const SZ: usize>(bytes: &[u8]) -> Option<[u8; SZ]> {
    if bytes.len() != SZ {
        return None;
    }
    let mut array = [0u8; SZ];
    array.copy_from_slice(&bytes[..SZ]);
    Some(array)
}
