//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    io,
    io::{Read, Seek},
    ops::RangeInclusive,
};

use tari_engine_types::confidential::ValueLookupTable;

use crate::value_lookup::header::LookupHeader;

/// Size of the buffer used to read ahead in the value lookup table. Must be a multiple of 32.
const LOOKAHEAD_BUFFER_SIZE: usize = 1024 * 1024;

pub struct IoReaderValueLookup<'a, R> {
    reader: &'a mut R,
    buffer: Vec<u8>,
    header: LookupHeader,
    pos: usize,
    last_value: u64,
}

impl<'a, R: Read + Seek> IoReaderValueLookup<'a, R> {
    pub fn load(reader: &'a mut R) -> io::Result<Self> {
        let header = LookupHeader::read(reader)?;
        Ok(Self {
            reader,
            buffer: Vec::with_capacity(LOOKAHEAD_BUFFER_SIZE),
            header,
            pos: 0,
            last_value: 0,
        })
    }

    fn seek_and_buffer_to_value(&mut self, value: u64) -> io::Result<()> {
        self.reader
            .seek(io::SeekFrom::Start(value * 32 + LookupHeader::SIZE as u64))?;
        self.buffer_next()?;
        Ok(())
    }

    fn buffer_next(&mut self) -> io::Result<()> {
        self.buffer.clear();
        self.reader
            .take(LOOKAHEAD_BUFFER_SIZE as u64)
            .read_to_end(&mut self.buffer)?;
        self.pos = 0;
        Ok(())
    }

    fn read_next(&mut self) -> io::Result<Option<[u8; 32]>> {
        if self.buffer.is_empty() {
            return Ok(None);
        }
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&self.buffer[self.pos * 32..(self.pos + 1) * 32]);
        self.pos += 1;
        if self.remaining_buffer() == 0 {
            self.buffer_next()?;
        }
        Ok(Some(buf))
    }

    fn buffer_pos(&self) -> usize {
        self.pos * 32
    }

    fn remaining_buffer(&self) -> usize {
        self.buffer.len() - self.buffer_pos()
    }

    /// Returns the supported value range of the lookup table
    pub fn range(&self) -> RangeInclusive<u64> {
        RangeInclusive::new(self.header.min, self.header.max)
    }
}

impl<'a, R: Read + Seek> ValueLookupTable for IoReaderValueLookup<'a, R> {
    type Error = io::Error;

    fn lookup(&mut self, value: u64) -> Result<Option<[u8; 32]>, Self::Error> {
        if !self.header.is_in_range(value) {
            return Ok(None);
        }

        if self.buffer.is_empty() || value != self.last_value + 1 {
            self.seek_and_buffer_to_value(value)?;
        }
        self.last_value = value;

        self.read_next()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use rand::{rngs::OsRng, Rng};

    use super::*;
    use crate::value_lookup::header::LOOKUP_HEADER_LEADING_BYTES;

    fn generate_lookup_data(min: u64, max: u64) -> Vec<u8> {
        let mut data = Vec::with_capacity(LOOKAHEAD_BUFFER_SIZE + 32 * (max - min + 1) as usize);
        data.extend_from_slice(LOOKUP_HEADER_LEADING_BYTES);
        data.extend_from_slice(&min.to_le_bytes());
        data.extend_from_slice(&max.to_le_bytes());
        for i in min..=max {
            let byte = i % u64::from(u8::MAX);
            data.extend_from_slice(&[byte as u8; 32]);
        }
        data
    }

    #[test]
    fn it_reads_the_header_correctly() {
        let lookup_data = generate_lookup_data(0, 10);
        let mut reader = Cursor::new(lookup_data.as_slice());
        let lookup = IoReaderValueLookup::load(&mut reader).unwrap();
        assert_eq!(lookup.range(), 0..=10);
    }

    #[test]
    fn it_reads_from_the_data_file_until_end() {
        const NUM: u64 = (LOOKAHEAD_BUFFER_SIZE + 11) as u64;
        let lookup_data = generate_lookup_data(0, NUM);
        let mut reader = Cursor::new(lookup_data.as_slice());
        let mut lookup = IoReaderValueLookup::load(&mut reader).unwrap();
        for v in 0..=NUM {
            let value = lookup.lookup(v).unwrap().unwrap();
            let byte = v % u64::from(u8::MAX);
            assert_eq!(value, [byte as u8; 32], "Failed at value {}", v);
        }
    }

    #[test]
    fn it_reads_non_sequential_values() {
        const NUM: u64 = (LOOKAHEAD_BUFFER_SIZE + 11) as u64;
        let lookup_data = generate_lookup_data(0, NUM);
        let mut reader = Cursor::new(lookup_data.as_slice());
        let mut lookup = IoReaderValueLookup::load(&mut reader).unwrap();
        for _ in 0..=1000 {
            let v = OsRng.gen_range(0..=NUM);
            let value = lookup.lookup(v).unwrap().unwrap();
            let byte = v % u64::from(u8::MAX);
            assert_eq!(value, [byte as u8; 32], "Failed at value {}", v);
        }
    }
}
