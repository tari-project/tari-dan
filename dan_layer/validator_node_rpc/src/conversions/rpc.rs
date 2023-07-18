//  Copyright 2023 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::convert::{TryFrom, TryInto};

use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::consensus_models::SubstateRecord;
use tari_engine_types::substate::{SubstateAddress, SubstateValue};

use crate::proto;

impl TryFrom<proto::rpc::VnStateSyncResponse> for SubstateRecord {
    type Error = anyhow::Error;

    fn try_from(value: proto::rpc::VnStateSyncResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            address: SubstateAddress::from_bytes(&value.address)?,
            version: value.version,
            substate_value: SubstateValue::from_bytes(&value.substate)?,
            state_hash: Default::default(),

            created_at_epoch: Epoch(value.created_epoch),
            created_by_transaction: value.created_transaction.try_into()?,
            created_justify: value.created_justify.try_into()?,
            created_block: value.created_block.try_into()?,
            created_height: NodeHeight(value.created_height),

            destroyed_by_transaction: Some(value.destroyed_transaction)
                .filter(|v| !v.is_empty())
                .map(TryInto::try_into)
                .transpose()?,
            destroyed_justify: Some(value.destroyed_justify)
                .filter(|v| !v.is_empty())
                .map(TryInto::try_into)
                .transpose()?,
            destroyed_by_block: Some(value.destroyed_block)
                .filter(|v| !v.is_empty())
                .map(TryInto::try_into)
                .transpose()?,
            destroyed_at_epoch: value.destroyed_epoch.map(Into::into),
        })
    }
}

impl From<SubstateRecord> for proto::rpc::VnStateSyncResponse {
    fn from(value: SubstateRecord) -> Self {
        Self {
            address: value.address.to_bytes(),
            version: value.version,
            substate: value.substate_value.to_bytes(),

            created_transaction: value.created_by_transaction.as_bytes().to_vec(),
            created_justify: value.created_justify.as_bytes().to_vec(),
            created_block: value.created_block.as_bytes().to_vec(),
            created_height: value.created_height.as_u64(),
            created_epoch: value.created_at_epoch.as_u64(),

            destroyed_transaction: value
                .destroyed_by_transaction
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_default(),
            destroyed_justify: value
                .destroyed_justify
                .map(|id| id.as_bytes().to_vec())
                .unwrap_or_default(),
            destroyed_block: value
                .destroyed_by_block
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_default(),
            destroyed_epoch: value.destroyed_at_epoch.map(Into::into),
        }
    }
}
