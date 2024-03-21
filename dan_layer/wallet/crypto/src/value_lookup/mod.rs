//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod header;
pub use header::*;

mod io_reader_value_lookup;
use std::convert::Infallible;

pub use io_reader_value_lookup::*;
use tari_engine_types::confidential::ValueLookupTable;

#[derive(Clone)]
pub struct AlwaysMissLookupTable;

impl ValueLookupTable for AlwaysMissLookupTable {
    type Error = Infallible;

    fn lookup(&mut self, _value: u64) -> Result<Option<[u8; 32]>, Self::Error> {
        Ok(None)
    }
}
