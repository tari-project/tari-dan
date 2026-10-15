//  Copyright 2023 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::convert::{TryFrom, TryInto};

use anyhow::{Context, anyhow};
use tari_engine_types::{
    limits::MAX_CBOR_NESTING_DEPTH,
    published_template::PublishedTemplateMetadata,
    substate::{SubstateId, SubstateValue},
};
use tari_jellyfish::TreeHash;
use tari_ootle_common_types::{SubstateVersion, shard::Shard};
use tari_ootle_storage::consensus_models::{
    EpochCheckpoint,
    SubstateCreate,
    SubstateData,
    SubstateDestroy,
    SubstateUpdateProof,
    SubstateValueOrHash,
    TreeRootSummary,
};
use tari_template_lib::types::Hash32;

use crate::{
    encoding::{decode_from_slice, encode_to_vec},
    proto,
};

fn decode_template_metadata(bytes: &[u8]) -> Result<PublishedTemplateMetadata, anyhow::Error> {
    decode_from_slice(bytes).context("TemplateMetadata")
}

impl TryFrom<proto::rpc::SubstateCreate> for SubstateCreate {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::SubstateCreate) -> Result<Self, Self::Error> {
        Ok(Self {
            substate: value
                .substate
                .map(TryInto::try_into)
                .transpose()?
                .ok_or_else(|| anyhow!("substate not provided"))?,
        })
    }
}

impl From<SubstateCreate> for proto::rpc::SubstateCreate {
    fn from(value: SubstateCreate) -> Self {
        Self {
            substate: Some(value.substate.into()),
        }
    }
}

impl TryFrom<proto::rpc::SubstateDestroy> for SubstateDestroy {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::SubstateDestroy) -> Result<Self, Self::Error> {
        Ok(Self {
            substate_id: SubstateId::from_bytes(&value.substate_id)?,
            version: SubstateVersion::new(value.version),
        })
    }
}

impl From<SubstateDestroy> for proto::rpc::SubstateDestroy {
    fn from(value: SubstateDestroy) -> Self {
        Self {
            substate_id: value.substate_id.to_bytes(),
            version: value.version.as_u64(),
        }
    }
}

impl TryFrom<proto::rpc::SubstateUpdate> for SubstateUpdateProof {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::SubstateUpdate) -> Result<Self, Self::Error> {
        let update = value.update.ok_or_else(|| anyhow!("update not provided"))?;
        match update {
            proto::rpc::substate_update::Update::Create(create) => Ok(Self::Create(Box::new(create.try_into()?))),
            proto::rpc::substate_update::Update::Destroy(destroy) => Ok(Self::Destroy(destroy.try_into()?)),
        }
    }
}

impl From<SubstateUpdateProof> for proto::rpc::SubstateUpdate {
    fn from(value: SubstateUpdateProof) -> Self {
        let update = match value {
            SubstateUpdateProof::Create(create) => proto::rpc::substate_update::Update::Create((*create).into()),
            SubstateUpdateProof::Destroy(destroy) => proto::rpc::substate_update::Update::Destroy(destroy.into()),
        };

        Self { update: Some(update) }
    }
}

impl TryFrom<proto::rpc::SubstateData> for SubstateData {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::SubstateData) -> Result<Self, Self::Error> {
        let template_metadata = if value.template_metadata.is_empty() {
            None
        } else {
            Some(decode_template_metadata(&value.template_metadata)?)
        };

        Ok(Self {
            substate_id: SubstateId::from_bytes(&value.substate_id)?,
            version: SubstateVersion::new(value.version),
            value: value
                .substate_value_or_hash
                .ok_or_else(|| anyhow!("substate_value_or_hash not provided"))?
                .try_into()?,
            template_metadata,
        })
    }
}

impl From<SubstateData> for proto::rpc::SubstateData {
    fn from(value: SubstateData) -> Self {
        Self {
            substate_id: value.substate_id.to_bytes(),
            version: value.version.as_u64(),
            substate_value_or_hash: Some(value.value().into()),
            template_metadata: value
                .template_metadata
                .map(|m| encode_to_vec(&m).unwrap_or_default())
                .unwrap_or_default(),
        }
    }
}

// -------------------------------- SubstateValueOrHash -------------------------------- //

impl From<&SubstateValueOrHash> for proto::rpc::substate_data::SubstateValueOrHash {
    fn from(value: &SubstateValueOrHash) -> Self {
        match value {
            SubstateValueOrHash::Value(v) => proto::rpc::substate_data::SubstateValueOrHash::Value(v.to_bytes()),
            SubstateValueOrHash::Hash(h) => proto::rpc::substate_data::SubstateValueOrHash::Hash(h.to_vec()),
        }
    }
}

impl TryFrom<proto::rpc::substate_data::SubstateValueOrHash> for SubstateValueOrHash {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::substate_data::SubstateValueOrHash) -> Result<Self, Self::Error> {
        match value {
            proto::rpc::substate_data::SubstateValueOrHash::Value(v) => Ok(SubstateValueOrHash::Value(Box::new(
                SubstateValue::from_bytes(&v).context("SubstateValueOrHash::Value")?,
            ))),
            proto::rpc::substate_data::SubstateValueOrHash::Hash(h) => Ok(SubstateValueOrHash::Hash(
                Hash32::try_from(h).context("SubstateValueOrHash::Hash")?,
            )),
        }
    }
}

//---------------------------------- EpochCheckpoint --------------------------------------------//

impl TryFrom<proto::rpc::EpochCheckpoint> for EpochCheckpoint {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::EpochCheckpoint) -> Result<Self, Self::Error> {
        // Defensive check to mitigate DoS attacks
        if value.shard_tree_summary.len() > 100_000 {
            return Err(anyhow!("too many shard roots (num={})", value.shard_tree_summary.len()));
        }

        let shard_tree_summary = value
            .shard_tree_summary
            .into_iter()
            .map(|(k, v)| v.try_into().map(|s| (Shard::from(k), s)))
            .collect::<Result<_, _>>()?;

        Ok(Self::new(
            // `CommandCommitProof` is a foreign serde-only type; serialize/deserialize via
            // tari_bor::serde_codec directly to match the encoding produced by
            // `encode_to_vec`/`tari_bor::adapters::serde_bridge`.
            tari_bor::serde_codec::from_slice_with_max_depth(&value.proof, MAX_CBOR_NESTING_DEPTH)
                .map_err(|e| anyhow!("Failed to decode CommandCommitProof: {e}"))?,
            shard_tree_summary,
            if value.header_metadata.is_empty() {
                None
            } else {
                Some(decode_from_slice(&value.header_metadata).context("BlockHeaderMetadata")?)
            },
        ))
    }
}

impl From<EpochCheckpoint> for proto::rpc::EpochCheckpoint {
    fn from(value: EpochCheckpoint) -> Self {
        Self {
            proof: tari_bor::serde_codec::to_vec(value.proof()).expect("Failed to encode CommandCommitProof"),
            shard_tree_summary: value
                .shard_tree_summary()
                .iter()
                .map(|(k, v)| (k.as_u32(), v.into()))
                .collect(),
            header_metadata: value
                .header_metadata()
                .map(|metadata| encode_to_vec(metadata).expect("Failed to encode BlockHeaderMetadata"))
                .unwrap_or_default(),
        }
    }
}

// -------------------------------- ConsensusState -------------------------------- //
impl From<tari_consensus::hotstuff::ConsensusCurrentState> for proto::rpc::ConsensusState {
    fn from(value: tari_consensus::hotstuff::ConsensusCurrentState) -> Self {
        use tari_consensus::hotstuff::ConsensusCurrentState::*;
        match value {
            Initialising => proto::rpc::ConsensusState::Initialising,
            Idle => proto::rpc::ConsensusState::Idle,
            CheckSync => proto::rpc::ConsensusState::CheckSync,
            Syncing => proto::rpc::ConsensusState::Syncing,
            Running => proto::rpc::ConsensusState::Running,
            Sleeping => proto::rpc::ConsensusState::Sleeping,
            Shutdown => proto::rpc::ConsensusState::Shutdown,
        }
    }
}

impl From<proto::rpc::ConsensusState> for tari_consensus::hotstuff::ConsensusCurrentState {
    fn from(value: proto::rpc::ConsensusState) -> Self {
        use tari_consensus::hotstuff::ConsensusCurrentState::*;
        match value {
            proto::rpc::ConsensusState::Initialising => Initialising,
            proto::rpc::ConsensusState::Idle => Idle,
            proto::rpc::ConsensusState::CheckSync => CheckSync,
            proto::rpc::ConsensusState::Syncing => Syncing,
            proto::rpc::ConsensusState::Running => Running,
            proto::rpc::ConsensusState::Sleeping => Sleeping,
            proto::rpc::ConsensusState::Shutdown => Shutdown,
        }
    }
}

// -------------------------------- TreeRootSummary -------------------------------- //
impl TryFrom<proto::rpc::TreeRootSummary> for TreeRootSummary {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::TreeRootSummary) -> Result<Self, Self::Error> {
        Ok(Self {
            root_hash: TreeHash::try_from_bytes(&value.root_hash).context("TreeRootSummary::root_hash")?,
            state_version: value.state_version,
        })
    }
}

impl From<&TreeRootSummary> for proto::rpc::TreeRootSummary {
    fn from(value: &TreeRootSummary) -> Self {
        Self {
            root_hash: value.root_hash.as_slice().to_vec(),
            state_version: value.state_version,
        }
    }
}
