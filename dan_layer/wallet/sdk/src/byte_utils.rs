//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub fn copy_fixed<const SZ: usize>(bytes: &[u8]) -> [u8; SZ] {
    let mut array = [0u8; SZ];
    array.copy_from_slice(&bytes[..SZ]);
    array
}
