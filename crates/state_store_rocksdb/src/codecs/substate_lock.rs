//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::io::Write;

use anyhow::anyhow;
use tari_consensus_types::BlockId;
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_ootle_transaction::TransactionId;
use tari_sidechain::FixedHash;

use crate::{
    codecs::{DbDecoder, DbEncoder, SubstateIdCodec},
    column_families::substate_locks::legacy::SubstateLockKey,
    error::RocksDbStorageError,
    utils::take_fixed,
};

pub struct SubstateLockKeyCodec<T> {
    substate_id_codec: SubstateIdCodec,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> SubstateLockKeyCodec<T> {
    fn decode_transaction_id(&self, bytes: &[u8]) -> Result<(TransactionId, usize), RocksDbStorageError> {
        let buf: [u8; TransactionId::byte_size()] =
            take_fixed(bytes).ok_or_else(|| RocksDbStorageError::DecodeError {
                source: anyhow!("SubstateLockKeyCodec: Invalid bytes for TransactionId"),
            })?;
        Ok((TransactionId::new(buf), TransactionId::byte_size()))
    }

    fn decode_block_id(&self, bytes: &[u8]) -> Result<(BlockId, usize), RocksDbStorageError> {
        let buf: [u8; BlockId::byte_size()] = take_fixed(bytes).ok_or_else(|| RocksDbStorageError::DecodeError {
            source: anyhow!("SubstateLockKeyCodec: Invalid bytes for BlockId"),
        })?;
        Ok((BlockId::new(FixedHash::new(buf)), BlockId::byte_size()))
    }

    fn decode_substate_id(&self, bytes: &[u8]) -> Result<(SubstateId, usize), RocksDbStorageError> {
        self.substate_id_codec
            .decode(bytes)
            .map_err(|e| RocksDbStorageError::DecodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to decode SubstateId: {}", e),
            })
    }

    fn decode_block_height(&self, bytes: &[u8]) -> Result<(NodeHeight, usize), RocksDbStorageError> {
        let height: [u8; 8] = take_fixed(bytes).ok_or_else(|| RocksDbStorageError::DecodeError {
            source: anyhow!("SubstateLockKeyCodec: Invalid bytes for NodeHeight"),
        })?;
        Ok((NodeHeight(u64::from_be_bytes(height)), 8))
    }

    fn decode_block_epoch(&self, bytes: &[u8]) -> Result<(Epoch, usize), RocksDbStorageError> {
        let epoch: [u8; 8] = take_fixed(bytes).ok_or_else(|| RocksDbStorageError::DecodeError {
            source: anyhow!("SubstateLockKeyCodec: Invalid bytes for Epoch"),
        })?;
        Ok((Epoch(u64::from_be_bytes(epoch)), 8))
    }

    fn encode_block_epoch<W: Write>(&self, value: &SubstateLockKey, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(&value.block_epoch.to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write block_epoch: {}", e),
            })
    }

    fn decode_grant_seq(&self, bytes: &[u8]) -> Result<(u32, usize), RocksDbStorageError> {
        let grant_seq: [u8; 4] = take_fixed(bytes).ok_or_else(|| RocksDbStorageError::DecodeError {
            source: anyhow!("SubstateLockKeyCodec: Invalid bytes for grant_seq"),
        })?;
        Ok((u32::from_be_bytes(grant_seq), 4))
    }

    fn encode_grant_seq<W: Write>(&self, value: &SubstateLockKey, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(&value.grant_seq.to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write grant_seq: {}", e),
            })
    }

    fn get_encoded_len(&self, value: &SubstateLockKey) -> Result<usize, RocksDbStorageError> {
        let len = BlockId::byte_size() + // block_id
            self.substate_id_codec.encode_len(&value.substate_id)? + // substate_id
            TransactionId::byte_size() + // transaction_id
            8 + // block_epoch
            8 + // block_height
            4; // grant_seq
        Ok(len)
    }
}

impl DbEncoder<SubstateLockKey> for SubstateLockKeyCodec<(TransactionId, SubstateId, BlockId, Epoch, NodeHeight, u32)> {
    fn encode_len(&self, value: &SubstateLockKey) -> Result<usize, RocksDbStorageError> {
        self.get_encoded_len(value)
    }

    fn encode_into<W: Write>(&self, value: &SubstateLockKey, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(value.transaction_id.as_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write transaction_id: {}", e),
            })?;
        self.substate_id_codec.encode_into(&value.substate_id, writer)?;
        writer
            .write_all(value.block_id.as_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write block_id: {}", e),
            })?;
        self.encode_block_epoch(value, writer)?;
        writer
            .write_all(&value.block_height.as_u64().to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write block_height: {}", e),
            })?;
        self.encode_grant_seq(value, writer)?;
        Ok(())
    }
}

impl DbDecoder<SubstateLockKey> for SubstateLockKeyCodec<(TransactionId, SubstateId, BlockId, Epoch, NodeHeight, u32)> {
    fn decode(&self, bytes: &[u8]) -> Result<(SubstateLockKey, usize), RocksDbStorageError> {
        let mut offset = 0;
        let (transaction_id, n) = self.decode_transaction_id(&bytes[offset..])?;
        offset += n;
        let (substate_id, n) = self.decode_substate_id(&bytes[offset..])?;
        offset += n;
        let (block_id, n) = self.decode_block_id(&bytes[offset..])?;
        offset += n;
        let (block_epoch, n) = self.decode_block_epoch(&bytes[offset..])?;
        offset += n;
        let (block_height, n) = self.decode_block_height(&bytes[offset..])?;
        offset += n;
        let (grant_seq, n) = self.decode_grant_seq(&bytes[offset..])?;
        offset += n;
        Ok((
            SubstateLockKey {
                block_id,
                substate_id,
                transaction_id,
                block_epoch,
                block_height,
                grant_seq,
            },
            offset,
        ))
    }
}

impl DbEncoder<SubstateLockKey> for SubstateLockKeyCodec<(BlockId, SubstateId, TransactionId, Epoch, NodeHeight, u32)> {
    fn encode_len(&self, value: &SubstateLockKey) -> Result<usize, RocksDbStorageError> {
        self.get_encoded_len(value)
    }

    fn encode_into<W: Write>(&self, value: &SubstateLockKey, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(value.block_id.as_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write block_id: {}", e),
            })?;
        self.substate_id_codec.encode_into(&value.substate_id, writer)?;
        writer
            .write_all(value.transaction_id.as_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write transaction_id: {}", e),
            })?;
        self.encode_block_epoch(value, writer)?;
        writer
            .write_all(&value.block_height.as_u64().to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write block_height: {}", e),
            })?;
        self.encode_grant_seq(value, writer)?;
        Ok(())
    }
}

impl DbDecoder<SubstateLockKey> for SubstateLockKeyCodec<(BlockId, SubstateId, TransactionId, Epoch, NodeHeight, u32)> {
    fn decode(&self, bytes: &[u8]) -> Result<(SubstateLockKey, usize), RocksDbStorageError> {
        let mut offset = 0;
        let (block_id, n) = self.decode_block_id(&bytes[offset..])?;
        offset += n;
        let (substate_id, n) = self.decode_substate_id(&bytes[offset..])?;
        offset += n;
        let (transaction_id, n) = self.decode_transaction_id(&bytes[offset..])?;
        offset += n;
        let (block_epoch, n) = self.decode_block_epoch(&bytes[offset..])?;
        offset += n;
        let (block_height, n) = self.decode_block_height(&bytes[offset..])?;
        offset += n;
        let (grant_seq, n) = self.decode_grant_seq(&bytes[offset..])?;
        offset += n;

        Ok((
            SubstateLockKey {
                block_id,
                substate_id,
                transaction_id,
                block_epoch,
                block_height,
                grant_seq,
            },
            offset,
        ))
    }
}

impl DbEncoder<SubstateLockKey> for SubstateLockKeyCodec<(SubstateId, TransactionId, BlockId, Epoch, NodeHeight, u32)> {
    fn encode_len(&self, value: &SubstateLockKey) -> Result<usize, RocksDbStorageError> {
        self.get_encoded_len(value)
    }

    fn encode_into<W: Write>(&self, value: &SubstateLockKey, writer: &mut W) -> Result<(), RocksDbStorageError> {
        writer
            .write_all(&self.substate_id_codec.encode(&value.substate_id)?)
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write substate_id: {}", e),
            })?;
        writer
            .write_all(value.transaction_id.as_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write transaction_id: {}", e),
            })?;
        writer
            .write_all(value.block_id.as_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write block_id: {}", e),
            })?;
        self.encode_block_epoch(value, writer)?;
        writer
            .write_all(&value.block_height.as_u64().to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow!("SubstateLockKeyCodec: Failed to write block_height: {}", e),
            })?;
        self.encode_grant_seq(value, writer)?;
        Ok(())
    }
}

impl DbDecoder<SubstateLockKey> for SubstateLockKeyCodec<(SubstateId, TransactionId, BlockId, Epoch, NodeHeight, u32)> {
    fn decode(&self, bytes: &[u8]) -> Result<(SubstateLockKey, usize), RocksDbStorageError> {
        let mut offset = 0;
        let (substate_id, n) = self.decode_substate_id(&bytes[offset..])?;
        offset += n;
        let (transaction_id, n) = self.decode_transaction_id(&bytes[offset..])?;
        offset += n;
        let (block_id, n) = self.decode_block_id(&bytes[offset..])?;
        offset += n;
        let (block_epoch, n) = self.decode_block_epoch(&bytes[offset..])?;
        offset += n;
        let (block_height, n) = self.decode_block_height(&bytes[offset..])?;
        offset += n;
        let (grant_seq, n) = self.decode_grant_seq(&bytes[offset..])?;
        offset += n;

        Ok((
            SubstateLockKey {
                block_id,
                substate_id,
                transaction_id,
                block_epoch,
                block_height,
                grant_seq,
            },
            offset,
        ))
    }
}

impl<T> Default for SubstateLockKeyCodec<T> {
    fn default() -> Self {
        Self {
            substate_id_codec: SubstateIdCodec,
            _phantom: Default::default(),
        }
    }
}
