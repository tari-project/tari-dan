//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub trait ValueLookupTable {
    type Error;
    fn lookup(&mut self, value: u64) -> Result<Option<[u8; 32]>, Self::Error>;
}
