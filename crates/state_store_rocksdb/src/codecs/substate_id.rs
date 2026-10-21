//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::io::{Cursor, Write};

use borsh::BorshDeserialize;
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::VersionedSubstateIdRef;

use crate::{
    codecs::{DbDecoder, DbEncoder, EncodeVec},
    error::RocksDbStorageError,
};

#[derive(Clone, Copy, Default)]
pub struct SubstateIdCodec;

impl SubstateIdCodec {
    fn get_substate_id_encoded_len(self, value: &SubstateId) -> Result<usize, RocksDbStorageError> {
        let len = borsh::object_length(value).map_err(|e| RocksDbStorageError::EncodeError { source: e.into() })?;
        Ok(len)
    }

    fn encode_substate_id_into<W: Write>(self, value: &SubstateId, writer: &mut W) -> Result<(), RocksDbStorageError> {
        borsh::to_writer(writer, value).map_err(|e| RocksDbStorageError::EncodeError { source: e.into() })?;
        Ok(())
    }
}

impl DbEncoder<SubstateId> for SubstateIdCodec {
    fn encode_len(&self, value: &SubstateId) -> Result<usize, RocksDbStorageError> {
        self.get_substate_id_encoded_len(value)
    }

    fn encode_into<W: Write>(&self, value: &SubstateId, writer: &mut W) -> Result<(), RocksDbStorageError> {
        self.encode_substate_id_into(value, writer)
    }
}

impl DbDecoder<SubstateId> for SubstateIdCodec {
    fn decode(&self, bytes: &[u8]) -> Result<(SubstateId, usize), RocksDbStorageError> {
        // We use deserialize_reader (rather than from_slice) so that no error is returned if there
        // are extra bytes in the buffer — this codec is used as a prefix in composite keys.
        // The cursor tracks how many bytes borsh consumed.
        let mut cursor = Cursor::new(bytes);
        let value = SubstateId::deserialize_reader(&mut cursor)
            .map_err(|e| RocksDbStorageError::DecodeError { source: e.into() })?;
        Ok((value, cursor.position() as usize))
    }
}

impl<'a> DbEncoder<&'a SubstateId> for SubstateIdCodec {
    fn encode_len(&self, value: &&'a SubstateId) -> Result<usize, RocksDbStorageError> {
        self.get_substate_id_encoded_len(value)
    }

    fn encode_into<W: Write>(&self, value: &&'a SubstateId, writer: &mut W) -> Result<(), RocksDbStorageError> {
        self.encode_substate_id_into(value, writer)
    }

    fn encode(&self, value: &&'a SubstateId) -> Result<EncodeVec, RocksDbStorageError> {
        SubstateIdCodec.encode(*value)
    }
}

impl<'a> DbEncoder<VersionedSubstateIdRef<'a>> for SubstateIdCodec {
    fn encode_len(&self, value: &VersionedSubstateIdRef<'a>) -> Result<usize, RocksDbStorageError> {
        self.get_substate_id_encoded_len(value.substate_id())
            .map(|len| len + size_of::<u32>())
    }

    fn encode_into<W: Write>(
        &self,
        value: &VersionedSubstateIdRef<'a>,
        writer: &mut W,
    ) -> Result<(), RocksDbStorageError> {
        self.encode_substate_id_into(value.substate_id(), writer)?;
        writer
            .write_all(&value.version().to_be_bytes())
            .map_err(|e| RocksDbStorageError::EncodeError {
                source: anyhow::anyhow!("VersionedSubstateIdRefCodec: Failed to write version: {}", e),
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::BlockId;
    use tari_ootle_common_types::SubstateVersion;
    use tari_sidechain::FixedHash;
    use tari_template_lib_types::{ComponentAddress, ObjectKey};

    use super::*;
    use crate::{
        codecs::{BlockDiffKeyCodec, SubstateIdBlockIdVersionSeq},
        column_families::block_diff::legacy::BlockDiffKey,
    };

    fn new_substate_id(seed: u8) -> SubstateId {
        let component = ComponentAddress::from_array([seed; ObjectKey::LENGTH]);
        SubstateId::Component(component)
    }

    fn new_block_id(seed: u8) -> BlockId {
        BlockId::new(FixedHash::from([seed; FixedHash::byte_size()]))
    }

    #[test]
    fn encode_decode() {
        let substate_id = new_substate_id(1);
        let codec = SubstateIdCodec;
        let encoded = codec.encode(&substate_id).unwrap();
        let decoded: SubstateId = codec.decode_exact(&encoded).unwrap();
        assert_eq!(substate_id, decoded);
    }

    #[test]
    fn check_with_prefix() {
        // This codec happens to encode {len}||{substate_id} as a prefix
        let block_diff_codec = BlockDiffKeyCodec::<SubstateIdBlockIdVersionSeq>::default();
        let key = BlockDiffKey {
            block_id: new_block_id(1),
            substate_id: new_substate_id(2),
            version: SubstateVersion::new(3),
            is_up: true,
            sequence: 0,
        };

        let compound_encoding = block_diff_codec.encode(&key).unwrap();
        let codec = SubstateIdCodec;
        let encoded = codec.encode(&key.substate_id).unwrap();
        assert_eq!(&compound_encoding.as_slice()[..encoded.len()], encoded.as_slice());
        let decoded = block_diff_codec.decode_exact(&compound_encoding).unwrap();
        assert_eq!(key.substate_id, decoded.substate_id);
        assert_eq!(key.version, decoded.version);
        assert_eq!(key.is_up, decoded.is_up);
        assert_eq!(key.sequence, decoded.sequence);
        assert_eq!(key.block_id, decoded.block_id);
    }
}
