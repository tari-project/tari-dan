//  Copyright 2023 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::convert::{TryFrom, TryInto};

use anyhow::anyhow;
use tari_dan_storage::consensus_models::{
    EpochCheckpoint,
    StateTransition,
    StateTransitionId,
    SubstateCreatedProof,
    SubstateData,
    SubstateDestroyedProof,
    SubstateUpdate,
};
use tari_engine_types::substate::{SubstateId, SubstateValue};

use crate::proto;

impl TryFrom<proto::rpc::SubstateCreatedProof> for SubstateCreatedProof {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::SubstateCreatedProof) -> Result<Self, Self::Error> {
        Ok(Self {
            substate: value
                .substate
                .map(TryInto::try_into)
                .transpose()?
                .ok_or_else(|| anyhow!("substate not provided"))?,
            created_qc: value
                .created_justify
                .map(TryInto::try_into)
                .transpose()?
                .ok_or_else(|| anyhow!("created_justify not provided"))?,
        })
    }
}

impl From<SubstateCreatedProof> for proto::rpc::SubstateCreatedProof {
    fn from(value: SubstateCreatedProof) -> Self {
        Self {
            substate: Some(value.substate.into()),
            created_justify: Some((&value.created_qc).into()),
        }
    }
}

impl TryFrom<proto::rpc::SubstateDestroyedProof> for SubstateDestroyedProof {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::SubstateDestroyedProof) -> Result<Self, Self::Error> {
        Ok(Self {
            substate_id: SubstateId::from_bytes(&value.substate_id)?,
            version: value.version,
            justify: value
                .destroyed_justify
                .map(TryInto::try_into)
                .transpose()?
                .ok_or_else(|| anyhow!("destroyed_justify not provided"))?,
            destroyed_by_transaction: value.destroyed_by_transaction.try_into()?,
        })
    }
}

impl From<SubstateDestroyedProof> for proto::rpc::SubstateDestroyedProof {
    fn from(value: SubstateDestroyedProof) -> Self {
        Self {
            substate_id: value.substate_id.to_bytes(),
            version: value.version,
            destroyed_justify: Some((&value.justify).into()),
            destroyed_by_transaction: value.destroyed_by_transaction.as_bytes().to_vec(),
        }
    }
}

impl TryFrom<proto::rpc::SubstateUpdate> for SubstateUpdate {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::SubstateUpdate) -> Result<Self, Self::Error> {
        let update = value.update.ok_or_else(|| anyhow!("update not provided"))?;
        match update {
            proto::rpc::substate_update::Update::Create(substate_proof) => Ok(Self::Create(substate_proof.try_into()?)),
            proto::rpc::substate_update::Update::Destroy(proof) => Ok(Self::Destroy(proof.try_into()?)),
        }
    }
}

impl From<SubstateUpdate> for proto::rpc::SubstateUpdate {
    fn from(value: SubstateUpdate) -> Self {
        let update = match value {
            SubstateUpdate::Create(proof) => proto::rpc::substate_update::Update::Create(proof.into()),
            SubstateUpdate::Destroy(proof) => proto::rpc::substate_update::Update::Destroy(proof.into()),
        };

        Self { update: Some(update) }
    }
}

impl TryFrom<proto::rpc::SubstateData> for SubstateData {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::SubstateData) -> Result<Self, Self::Error> {
        Ok(Self {
            substate_id: SubstateId::from_bytes(&value.substate_id)?,
            version: value.version,
            substate_value: SubstateValue::from_bytes(&value.substate_value)?,
            created_by_transaction: value.created_transaction.try_into()?,
        })
    }
}

impl From<SubstateData> for proto::rpc::SubstateData {
    fn from(value: SubstateData) -> Self {
        Self {
            substate_id: value.substate_id.to_bytes(),
            version: value.version,
            substate_value: value.substate_value.to_bytes(),
            created_transaction: value.created_by_transaction.as_bytes().to_vec(),
        }
    }
}

//---------------------------------- StateTransition --------------------------------------------//

impl TryFrom<proto::rpc::StateTransition> for StateTransition {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::StateTransition) -> Result<Self, Self::Error> {
        let id = StateTransitionId::from_bytes(&value.id)
            .ok_or_else(|| anyhow::anyhow!("Invalid state transition id bytes"))?;
        let update = value
            .update
            .ok_or_else(|| anyhow::anyhow!("Missing state transition update"))?;
        let update = SubstateUpdate::try_from(update)?;
        Ok(Self { id, update })
    }
}

impl From<StateTransition> for proto::rpc::StateTransition {
    fn from(value: StateTransition) -> Self {
        Self {
            id: value.id.as_bytes().to_vec(),
            update: Some(value.update.into()),
        }
    }
}

//---------------------------------- EpochCheckpoint --------------------------------------------//

impl TryFrom<proto::rpc::EpochCheckpoint> for EpochCheckpoint {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::EpochCheckpoint) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value.block.ok_or_else(|| anyhow!("block not provided"))?.try_into()?,
            value.qcs.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
        ))
    }
}

impl From<EpochCheckpoint> for proto::rpc::EpochCheckpoint {
    fn from(value: EpochCheckpoint) -> Self {
        Self {
            block: Some(value.block().into()),
            qcs: value.qcs().into_iter().map(Into::into).collect(),
        }
    }
}
