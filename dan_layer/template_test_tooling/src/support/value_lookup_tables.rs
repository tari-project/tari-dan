//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::convert::Infallible;

use tari_engine_types::confidential::ValueLookupTable;

#[derive(Clone)]
pub struct AlwaysMissLookupTable;

impl ValueLookupTable for AlwaysMissLookupTable {
    type Error = Infallible;

    fn lookup(&mut self, _value: u64) -> Result<Option<[u8; 32]>, Self::Error> {
        Ok(None)
    }
}
