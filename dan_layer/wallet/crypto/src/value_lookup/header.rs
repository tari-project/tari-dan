//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{io, mem::size_of};

pub const LOOKUP_HEADER_LEADING_BYTES: &[u8] = b"VLKP";

pub struct LookupHeader {
    pub min: u64,
    pub max: u64,
}

impl LookupHeader {
    pub const SIZE: usize = size_of::<u64>() * 2 + LOOKUP_HEADER_LEADING_BYTES.len();

    pub fn read<R: io::Read>(reader: &mut R) -> io::Result<Self> {
        let mut buf = [0u8; Self::SIZE];
        reader.read_exact(&mut buf)?;
        if &buf[0..4] != LOOKUP_HEADER_LEADING_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid lookup table header leading bytes",
            ));
        }
        let body = &buf[LOOKUP_HEADER_LEADING_BYTES.len()..];
        let mut u64_buf = [0u8; 8];
        u64_buf.copy_from_slice(&body[..8]);
        let min = u64::from_le_bytes(u64_buf);
        let mut u64_buf = [0u8; 8];
        u64_buf.copy_from_slice(&body[8..16]);
        let max = u64::from_le_bytes(u64_buf);
        Ok(Self { min, max })
    }

    pub fn is_in_range(&self, value: u64) -> bool {
        value >= self.min && value <= self.max
    }
}
