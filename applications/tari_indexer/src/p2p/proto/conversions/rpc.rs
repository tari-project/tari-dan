//  Copyright 2023 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::convert::{TryFrom, TryInto};

use anyhow;
use tari_dan_core::models::SubstateShardData;
use tari_engine_types::substate::{Substate, SubstateAddress};

use crate::p2p;

impl TryFrom<p2p::proto::rpc::VnStateSyncResponse> for SubstateShardData {
    type Error = anyhow::Error;

    fn try_from(value: p2p::proto::rpc::VnStateSyncResponse) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value.shard_id.try_into()?,
            SubstateAddress::from_bytes(&value.address)?,
            value.version,
            Substate::from_bytes(&value.substate)?,
            value.created_height.try_into()?,
            if value.destroyed_height == 0 {
                None
            } else {
                Some(value.destroyed_height.try_into()?)
            },
            value.created_node_hash.try_into()?,
            if value.destroyed_node_hash.is_empty() {
                None
            } else {
                Some(value.destroyed_node_hash.try_into()?)
            },
            value.created_payload_id.try_into()?,
            if value.destroyed_payload_id.is_empty() {
                None
            } else {
                Some(value.destroyed_payload_id.try_into()?)
            },
            value
                .created_justify
                .map(|v| v.try_into())
                .transpose()?
                .ok_or_else(|| anyhow::anyhow!("VnStateSyncResponse created_justify is required"))?,
            value.destroyed_justify.map(|v| v.try_into()).transpose()?,
        ))
    }
}

impl TryFrom<SubstateShardData> for p2p::proto::rpc::VnStateSyncResponse {
    type Error = anyhow::Error;

    fn try_from(value: SubstateShardData) -> Result<Self, Self::Error> {
        Ok(Self {
            shard_id: value.shard_id().as_bytes().to_vec(),
            version: value.version(),
            address: value.substate_address().to_bytes(),
            substate: value.substate().to_bytes(),
            created_height: value.created_height().as_u64(),
            destroyed_height: value.destroyed_height().map(|v| v.as_u64()).unwrap_or(0),
            created_node_hash: value.created_node_hash().as_bytes().to_vec(),
            destroyed_node_hash: value
                .destroyed_node_hash()
                .map(|v| v.as_bytes().to_vec())
                .unwrap_or_default(),
            created_payload_id: value.created_payload_id().as_bytes().to_vec(),
            destroyed_payload_id: value
                .destroyed_payload_id()
                .map(|v| v.as_bytes().to_vec())
                .unwrap_or_default(),
            created_justify: Some(value.created_justify().clone().try_into()?),
            destroyed_justify: value
                .destroyed_justify()
                .as_ref()
                .map(|v| v.clone().try_into())
                .transpose()?,
        })
    }
}
