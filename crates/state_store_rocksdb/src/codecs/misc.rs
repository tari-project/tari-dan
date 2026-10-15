//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::io::Write;

use tari_ootle_common_types::{Epoch, NodeHeight, shard::Shard};

use crate::{
    codecs::{DbDecoder, DbEncoder, EncodeVec},
    error::RocksDbStorageError,
    utils::take_fixed,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct UnitCodec;

impl DbEncoder<()> for UnitCodec {
    fn encode_len(&self, _value: &()) -> Result<usize, RocksDbStorageError> {
        Ok(0)
    }

    fn encode_into<W: Write>(&self, _value: &(), _writer: &mut W) -> Result<(), RocksDbStorageError> {
        Ok(())
    }

    fn encode(&self, _: &()) -> Result<EncodeVec, RocksDbStorageError> {
        Ok(EncodeVec::empty())
    }
}

impl DbDecoder<()> for UnitCodec {
    fn decode(&self, _bytes: &[u8]) -> Result<((), usize), RocksDbStorageError> {
        Ok(((), 0))
    }

    // Allow trailing bytes: this codec stands in as a value codec for key-only iterators,
    // where the underlying CF returns real value bytes that we want to discard.
    fn decode_exact(&self, _bytes: &[u8]) -> Result<(), RocksDbStorageError> {
        Ok(())
    }
}

#[derive(Default)]
pub struct BoolCodec;

impl DbEncoder<bool> for BoolCodec {
    fn encode_len(&self, _value: &bool) -> Result<usize, RocksDbStorageError> {
        Ok(1)
    }

    fn encode_into<W: Write>(&self, value: &bool, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(&[u8::from(*value)])
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow::anyhow!("BoolCodec: Failed to write bool: {}", e),
            })?;
        Ok(())
    }

    fn encode(&self, value: &bool) -> Result<EncodeVec, RocksDbStorageError> {
        Ok(EncodeVec::new_from_array([u8::from(*value)]))
    }
}

impl DbDecoder<bool> for BoolCodec {
    fn decode(&self, bytes: &[u8]) -> Result<(bool, usize), RocksDbStorageError> {
        let byte = bytes.first().ok_or_else(|| RocksDbStorageError::MalformedData {
            operation: "decode bool",
            details: "Not enough bytes for BoolCodec".to_string(),
        })?;
        Ok((*byte != 0, 1))
    }
}

pub type EpochCodec = NumberCodec<Epoch>;
pub type ShardCodec = NumberCodec<Shard>;
pub type NodeHeightCodec = NumberCodec<NodeHeight>;

#[derive(Debug, Clone, Copy)]
pub struct NumberCodec<T> {
    _phantom: std::marker::PhantomData<T>,
}

impl DbEncoder<u8> for NumberCodec<u8> {
    fn encode_len(&self, _value: &u8) -> Result<usize, RocksDbStorageError> {
        Ok(1)
    }

    fn encode_into<W: Write>(&self, value: &u8, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(&[*value])
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow::anyhow!("NumberCodec<u8>: Failed to write u8: {}", e),
            })?;
        Ok(())
    }

    fn encode(&self, value: &u8) -> Result<EncodeVec, RocksDbStorageError> {
        Ok(EncodeVec::new_from_array([*value]))
    }
}

impl DbDecoder<u8> for NumberCodec<u8> {
    fn decode(&self, bytes: &[u8]) -> Result<(u8, usize), RocksDbStorageError> {
        let byte = bytes.first().ok_or_else(|| RocksDbStorageError::MalformedData {
            operation: "decode u8",
            details: "Not enough bytes for NumberCodec<u8>".to_string(),
        })?;
        Ok((*byte, 1))
    }
}

impl DbEncoder<u32> for NumberCodec<u32> {
    fn encode_len(&self, _value: &u32) -> Result<usize, RocksDbStorageError> {
        Ok(size_of::<u32>())
    }

    fn encode_into<W: Write>(&self, value: &u32, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(&value.to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow::anyhow!("NumberCodec<u32>: Failed to write u32: {}", e),
            })?;
        Ok(())
    }

    fn encode(&self, value: &u32) -> Result<EncodeVec, RocksDbStorageError> {
        Ok(EncodeVec::new_from_array(value.to_be_bytes()))
    }
}

impl DbDecoder<u32> for NumberCodec<u32> {
    fn decode(&self, bytes: &[u8]) -> Result<(u32, usize), RocksDbStorageError> {
        let arr: [u8; 4] = take_fixed(bytes).ok_or_else(|| RocksDbStorageError::MalformedData {
            operation: "decode u32",
            details: "Invalid bytes size for NumberCodec<u32>".to_string(),
        })?;
        Ok((u32::from_be_bytes(arr), 4))
    }
}

impl DbEncoder<u64> for NumberCodec<u64> {
    fn encode_len(&self, _value: &u64) -> Result<usize, RocksDbStorageError> {
        Ok(size_of::<u64>())
    }

    fn encode_into<W: Write>(&self, value: &u64, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(&value.to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow::anyhow!("NumberCodec<u64>: Failed to write u64: {}", e),
            })?;
        Ok(())
    }

    fn encode(&self, value: &u64) -> Result<EncodeVec, RocksDbStorageError> {
        Ok(EncodeVec::new_from_array(value.to_be_bytes()))
    }
}
impl DbDecoder<u64> for NumberCodec<u64> {
    fn decode(&self, bytes: &[u8]) -> Result<(u64, usize), RocksDbStorageError> {
        let arr: [u8; 8] = take_fixed(bytes).ok_or_else(|| RocksDbStorageError::MalformedData {
            operation: "decode u64",
            details: "Invalid bytes size for NumberCodec<u64>".to_string(),
        })?;
        Ok((u64::from_be_bytes(arr), 8))
    }
}

impl DbEncoder<Epoch> for NumberCodec<Epoch> {
    fn encode_len(&self, _value: &Epoch) -> Result<usize, RocksDbStorageError> {
        Ok(size_of::<Epoch>())
    }

    fn encode_into<W: Write>(&self, value: &Epoch, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(&value.to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow::anyhow!("NumberCodec<Epoch>: Failed to write Epoch: {}", e),
            })?;
        Ok(())
    }

    fn encode(&self, epoch: &Epoch) -> Result<EncodeVec, RocksDbStorageError> {
        Ok(EncodeVec::new_from_array(epoch.to_be_bytes()))
    }
}

impl DbDecoder<Epoch> for NumberCodec<Epoch> {
    fn decode(&self, bytes: &[u8]) -> Result<(Epoch, usize), RocksDbStorageError> {
        let arr: [u8; 8] = take_fixed(bytes).ok_or_else(|| RocksDbStorageError::MalformedData {
            operation: "decode Epoch",
            details: "Invalid bytes size for NumberCodec<Epoch>".to_string(),
        })?;
        Ok((Epoch(u64::from_be_bytes(arr)), 8))
    }
}

impl DbEncoder<NodeHeight> for NumberCodec<NodeHeight> {
    fn encode_len(&self, _value: &NodeHeight) -> Result<usize, RocksDbStorageError> {
        Ok(size_of::<NodeHeight>())
    }

    fn encode_into<W: Write>(&self, value: &NodeHeight, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(&value.as_u64().to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow::anyhow!("NumberCodec<NodeHeight>: Failed to write NodeHeight: {}", e),
            })?;
        Ok(())
    }

    fn encode(&self, value: &NodeHeight) -> Result<EncodeVec, RocksDbStorageError> {
        Ok(EncodeVec::new_from_array(value.as_u64().to_be_bytes()))
    }
}

impl DbDecoder<NodeHeight> for NumberCodec<NodeHeight> {
    fn decode(&self, bytes: &[u8]) -> Result<(NodeHeight, usize), RocksDbStorageError> {
        let arr: [u8; 8] = take_fixed(bytes).ok_or_else(|| RocksDbStorageError::MalformedData {
            operation: "decode NodeHeight",
            details: "Invalid bytes size for NumberCodec<NodeHeight>".to_string(),
        })?;
        Ok((NodeHeight::from(u64::from_be_bytes(arr)), 8))
    }
}

impl DbEncoder<Shard> for NumberCodec<Shard> {
    fn encode_len(&self, _value: &Shard) -> Result<usize, RocksDbStorageError> {
        Ok(size_of::<Shard>())
    }

    fn encode_into<W: Write>(&self, value: &Shard, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(&value.as_u32().to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow::anyhow!("NumberCodec<Shard>: Failed to write Shard: {}", e),
            })?;
        Ok(())
    }

    fn encode(&self, shard: &Shard) -> Result<EncodeVec, RocksDbStorageError> {
        Ok(EncodeVec::new_from_array(shard.as_u32().to_be_bytes()))
    }
}

impl DbDecoder<Shard> for NumberCodec<Shard> {
    fn decode(&self, bytes: &[u8]) -> Result<(Shard, usize), RocksDbStorageError> {
        let arr: [u8; 4] = take_fixed(bytes).ok_or_else(|| RocksDbStorageError::MalformedData {
            operation: "decode Shard",
            details: "Invalid bytes size for NumberCodec<Shard>".to_string(),
        })?;
        Ok((u32::from_be_bytes(arr).into(), 4))
    }
}

impl<T> Default for NumberCodec<T> {
    fn default() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}
