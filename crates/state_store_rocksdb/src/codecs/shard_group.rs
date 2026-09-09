//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::io::Write;

use tari_ootle_common_types::ShardGroup;

use crate::{
    codecs::{DbDecoder, DbEncoder, EncodeVec, ShardCodec},
    error::RocksDbStorageError,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct ShardGroupCodec {
    inner: ShardCodec,
}

impl DbEncoder<ShardGroup> for ShardGroupCodec {
    fn encode_len(&self, value: &ShardGroup) -> Result<usize, RocksDbStorageError> {
        let start_len = self.inner.encode_len(&value.start())?;
        let end_len = self.inner.encode_len(&value.end())?;
        Ok(start_len + end_len)
    }

    fn encode_into<W: Write>(&self, value: &ShardGroup, writer: &mut W) -> Result<(), RocksDbStorageError> {
        self.inner.encode_into(&value.start(), writer)?;
        self.inner.encode_into(&value.end(), writer)?;
        Ok(())
    }

    fn encode(&self, value: &ShardGroup) -> Result<EncodeVec, RocksDbStorageError> {
        let start = self.inner.encode(&value.start())?;
        let end = self.inner.encode(&value.end())?;
        Ok(EncodeVec::from_slices(&[start.as_slice(), end.as_slice()]))
    }
}

impl DbDecoder<ShardGroup> for ShardGroupCodec {
    fn decode(&self, bytes: &[u8]) -> Result<(ShardGroup, usize), RocksDbStorageError> {
        let (start, n_start) = self.inner.decode(bytes)?;
        let (end, n_end) = self.inner.decode(&bytes[n_start..])?;
        let shard_group = ShardGroup::new_checked(start, end).ok_or_else(|| RocksDbStorageError::MalformedData {
            operation: "ShardGroupCodec::decode",
            details: format!(
                "start ({}) is greater than end_inclusive ({})",
                start.as_u32(),
                end.as_u32()
            ),
        })?;
        Ok((shard_group, n_start + n_end))
    }
}
