//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{
    quorum_certificate::QuorumCertificate,
    NodeHeight,
    PayloadId,
    ShardId,
    SubstateState,
    TreeNodeHash,
};
use tari_engine_types::substate::{Substate, SubstateAddress};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubstateShardData {
    shard_id: ShardId,
    address: SubstateAddress,
    version: u32,
    substate: Substate,
    created_height: NodeHeight,
    destroyed_height: Option<NodeHeight>,
    created_node_hash: TreeNodeHash,
    destroyed_node_hash: Option<TreeNodeHash>,
    created_payload_id: PayloadId,
    destroyed_payload_id: Option<PayloadId>,
    created_justify: Option<QuorumCertificate<PublicKey>>,
    destroyed_justify: Option<QuorumCertificate<PublicKey>>,
    created_fee_accrued: u64,
    destroyed_fee_accrued: u64,
}

impl SubstateShardData {
    pub fn new(
        shard_id: ShardId,
        address: SubstateAddress,
        version: u32,
        substate: Substate,
        created_height: NodeHeight,
        destroyed_height: Option<NodeHeight>,
        created_node_hash: TreeNodeHash,
        destroyed_node_hash: Option<TreeNodeHash>,
        created_payload_id: PayloadId,
        destroyed_payload_id: Option<PayloadId>,
        created_justify: Option<QuorumCertificate<PublicKey>>,
        destroyed_justify: Option<QuorumCertificate<PublicKey>>,
        created_fee_accrued: u64,
        destroyed_fee_accrued: u64,
    ) -> Self {
        Self {
            shard_id,
            address,
            version,
            substate,
            created_height,
            destroyed_height,
            created_node_hash,
            destroyed_node_hash,
            created_payload_id,
            destroyed_payload_id,
            created_justify,
            destroyed_justify,
            created_fee_accrued,
            destroyed_fee_accrued,
        }
    }

    pub fn shard_id(&self) -> ShardId {
        self.shard_id
    }

    pub fn substate_address(&self) -> &SubstateAddress {
        &self.address
    }

    pub fn substate(&self) -> &Substate {
        &self.substate
    }

    pub fn into_substate(self) -> Substate {
        self.substate
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn created_height(&self) -> NodeHeight {
        self.created_height
    }

    pub fn destroyed_height(&self) -> Option<NodeHeight> {
        self.destroyed_height
    }

    pub fn created_node_hash(&self) -> TreeNodeHash {
        self.created_node_hash
    }

    pub fn destroyed_node_hash(&self) -> Option<TreeNodeHash> {
        self.destroyed_node_hash
    }

    pub fn created_payload_id(&self) -> PayloadId {
        self.created_payload_id
    }

    pub fn destroyed_payload_id(&self) -> Option<PayloadId> {
        self.destroyed_payload_id
    }

    pub fn created_justify(&self) -> Option<&QuorumCertificate<PublicKey>> {
        self.created_justify.as_ref()
    }

    pub fn destroyed_justify(&self) -> &Option<QuorumCertificate<PublicKey>> {
        &self.destroyed_justify
    }

    pub fn created_fee_accrued(&self) -> u64 {
        self.created_fee_accrued
    }

    pub fn destroyed_fee_accrued(&self) -> u64 {
        self.destroyed_fee_accrued
    }

    pub fn into_substate_state(self) -> SubstateState {
        if let Some(payload_id) = self.destroyed_payload_id() {
            SubstateState::Down {
                deleted_by: payload_id,
                fees_accrued: 0,
            }
        } else {
            SubstateState::Up {
                address: self.address.clone(),
                created_by: self.created_payload_id(),
                data: self.into_substate(),
                fees_accrued: 0,
            }
        }
    }
}
