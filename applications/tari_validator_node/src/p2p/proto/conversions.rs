//  Copyright 2022, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    borrow::Borrow,
    convert::{TryFrom, TryInto},
};

use anyhow::anyhow;
use borsh::de::BorshDeserialize;
use chrono::{DateTime, NaiveDateTime, Utc};
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_comms::{peer_manager::IdentitySignature, types::CommsPublicKey};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    message::{DanMessage, NetworkAnnounce},
    models::{
        vote_message::VoteMessage,
        HotStuffMessage,
        HotStuffTreeNode,
        Node,
        QuorumCertificate,
        QuorumDecision,
        TariDanPayload,
        TreeNodeHash,
        ValidatorSignature,
    },
};
use tari_dan_engine::{
    state::{
        models::{KeyValue, StateOpLogEntry},
        DbStateOpLogEntry,
    },
    transaction::{Transaction, TransactionMeta},
};
use tari_template_lib::{args::Arg, Hash};

use crate::p2p::proto;

impl From<DanMessage<TariDanPayload, CommsPublicKey>> for proto::validator_node::DanMessage {
    fn from(msg: DanMessage<TariDanPayload, CommsPublicKey>) -> Self {
        match msg {
            DanMessage::HotStuffMessage(hot_stuff_msg) => Self {
                message: Some(proto::validator_node::dan_message::Message::HotStuff(
                    hot_stuff_msg.into(),
                )),
            },
            DanMessage::VoteMessage(vote_msg) => Self {
                message: Some(proto::validator_node::dan_message::Message::Vote(vote_msg.into())),
            },
            DanMessage::NewTransaction(transaction) => Self {
                message: Some(proto::validator_node::dan_message::Message::NewTransaction(
                    transaction.into(),
                )),
            },
            DanMessage::NetworkAnnounce(announce) => Self {
                message: Some(proto::validator_node::dan_message::Message::NetworkAnnounce(
                    announce.into(),
                )),
            },
        }
    }
}

impl TryFrom<proto::validator_node::DanMessage> for DanMessage<TariDanPayload, CommsPublicKey> {
    type Error = anyhow::Error;

    fn try_from(value: proto::validator_node::DanMessage) -> Result<Self, Self::Error> {
        let msg_type = value.message.ok_or_else(|| anyhow!("Message type not provided"))?;
        match msg_type {
            proto::validator_node::dan_message::Message::HotStuff(msg) => {
                Ok(DanMessage::HotStuffMessage(msg.try_into()?))
            },
            proto::validator_node::dan_message::Message::Vote(msg) => Ok(DanMessage::VoteMessage(msg.try_into()?)),
            proto::validator_node::dan_message::Message::NewTransaction(msg) => {
                Ok(DanMessage::NewTransaction(msg.try_into()?))
            },
            proto::validator_node::dan_message::Message::NetworkAnnounce(msg) => {
                Ok(DanMessage::NetworkAnnounce(msg.try_into()?))
            },
        }
    }
}

// -------------------------------- VoteMessage -------------------------------- //

impl From<VoteMessage> for proto::consensus::VoteMessage {
    fn from(msg: VoteMessage) -> Self {
        Self {
            local_node_hash: msg.local_node_hash().as_bytes().to_vec(),
            shard_id: msg.shard().as_bytes().to_vec(),
            decision: i32::from(msg.decision().as_u8()),
            all_shard_nodes: vec![], // TODO: msg.all_shard_nodes().iter().map(|n| n.into()).collect(),
            signature: msg.signature().to_bytes(),
        }
    }
}

impl TryFrom<proto::consensus::VoteMessage> for VoteMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::VoteMessage) -> Result<Self, Self::Error> {
        Ok(VoteMessage::with_signature(
            TreeNodeHash::try_from(value.local_node_hash)?,
            ShardId::from_bytes(&value.shard_id)?,
            QuorumDecision::from_u8(u8::try_from(value.decision)?)?,
            vec![], // TODO: value.all_shard_nodes,
            ValidatorSignature::from_bytes(&value.signature)?,
        ))
    }
}

// -------------------------------- HotstuffMessage -------------------------------- //

impl From<HotStuffMessage<TariDanPayload, CommsPublicKey>> for proto::consensus::HotStuffMessage {
    fn from(_source: HotStuffMessage<TariDanPayload, CommsPublicKey>) -> Self {
        todo!()
        // Self {
        //     message_type: i32::from(source.message_type().as_u8()),
        //     node: source.node().map(|n| n.clone().into()),
        //     justify: source.justify().map(|j| j.clone().into()),
        //     partial_sig: source.partial_sig().map(|s| s.clone().into()),
        //     view_number: source.view_number().as_u64(),
        //     node_hash: source
        //         .node_hash()
        //         .copied()
        //         .unwrap_or_else(TreeNodeHash::zero)
        //         .as_bytes()
        //         .to_vec(),
        //     checkpoint_signature: source.checkpoint_signature().map(Into::into),
        //     contract_id: source.contract_id().to_vec(),
        // }
    }
}

impl From<HotStuffTreeNode<CommsPublicKey>> for proto::consensus::HotStuffTreeNode {
    fn from(_source: HotStuffTreeNode<CommsPublicKey>) -> Self {
        todo!()
        // Self {
        //     parent: Vec::from(source.parent().as_bytes()),
        //     payload: Some(source.payload().clone().into()),
        //     height: source.height(),
        //     state_root: Vec::from(source.state_root().as_bytes()),
        // }
    }
}

// -------------------------------- NetworkAnnounce -------------------------------- //

impl<T: ByteArray> From<NetworkAnnounce<T>> for proto::network::NetworkAnnounce {
    fn from(msg: NetworkAnnounce<T>) -> Self {
        Self {
            identity: msg.identity.to_vec(),
            addresses: msg.addresses.into_iter().map(|a| a.to_vec()).collect(),
            identity_signature: Some(msg.identity_signature.into()),
        }
    }
}

impl<T: ByteArray> TryFrom<proto::network::NetworkAnnounce> for NetworkAnnounce<T> {
    type Error = anyhow::Error;

    fn try_from(value: proto::network::NetworkAnnounce) -> Result<Self, Self::Error> {
        Ok(NetworkAnnounce {
            identity: T::from_bytes(&value.identity)?,
            addresses: value
                .addresses
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            identity_signature: value
                .identity_signature
                .ok_or_else(|| anyhow!("Identity signature not provided"))?
                .try_into()?,
        })
    }
}

// -------------------------------- IdentitySignature -------------------------------- //

impl TryFrom<proto::network::IdentitySignature> for IdentitySignature {
    type Error = anyhow::Error;

    fn try_from(value: proto::network::IdentitySignature) -> Result<Self, Self::Error> {
        let version = u8::try_from(value.version).map_err(|_| anyhow!("Invalid identity signature version"))?;
        let signature = value
            .signature
            .ok_or_else(|| anyhow!("Identity signature missing signature"))?
            .try_into()?;
        let updated_at = NaiveDateTime::from_timestamp_opt(value.updated_at, 0)
            .ok_or_else(|| anyhow!("Invalid updated_at timestamp"))?;
        let updated_at = DateTime::<Utc>::from_utc(updated_at, Utc);

        Ok(IdentitySignature::new(
            version, // Signature::new(public_nonce, signature),
            signature, updated_at,
        ))
    }
}

impl<T: Borrow<IdentitySignature>> From<T> for proto::network::IdentitySignature {
    fn from(identity_sig: T) -> Self {
        let sig = identity_sig.borrow();
        proto::network::IdentitySignature {
            version: u32::from(sig.version()),
            signature: Some(sig.signature().into()),
            updated_at: sig.updated_at().timestamp(),
        }
    }
}

// -------------------------------- QuorumCertificate -------------------------------- //

impl From<QuorumCertificate> for proto::consensus::QuorumCertificate {
    fn from(_source: QuorumCertificate) -> Self {
        todo!()
        // Self {
        //     message_type: i32::from(source.message_type().as_u8()),
        //     node_hash: Vec::from(source.node_hash().as_bytes()),
        //     view_number: source.view_number().as_u64(),
        //     signature: source.signature().map(|s| s.clone().into()),
        // }
    }
}

impl From<ValidatorSignature> for proto::consensus::ValidatorSignature {
    fn from(_s: ValidatorSignature) -> Self {
        Self {}
    }
}

impl From<TariDanPayload> for proto::consensus::TariDanPayload {
    fn from(source: TariDanPayload) -> Self {
        Self {
            transaction: Some(source.transaction().clone().into()),
        }
    }
}

impl TryFrom<proto::consensus::HotStuffMessage> for HotStuffMessage<TariDanPayload, CommsPublicKey> {
    type Error = anyhow::Error;

    fn try_from(_value: proto::consensus::HotStuffMessage) -> Result<Self, Self::Error> {
        todo!()
        // let node_hash = if value.node_hash.is_empty() {
        //     None
        // } else {
        //     Some(TreeNodeHash::try_from(value.node_hash).map_err(|err| err.to_string())?)
        // };
        // Ok(Self::new(
        //     ViewId(value.view_number),
        //     HotStuffMessageType::try_from(u8::try_from(value.message_type).unwrap())?,
        //     value.justify.map(|j| j.try_into()).transpose()?,
        //     value.node.map(|n| n.try_into()).transpose()?,
        //     node_hash,
        //     value.partial_sig.map(|p| p.try_into()).transpose()?,
        //     value.checkpoint_signature.map(|p| p.try_into()).transpose()?,
        //     value
        //         .contract_id
        //         .try_into()
        //         .map_err(|err| format!("Not a valid contract ID:{}", err))?,
        // ))
    }
}

impl TryFrom<proto::consensus::QuorumCertificate> for QuorumCertificate {
    type Error = anyhow::Error;

    fn try_from(_value: proto::consensus::QuorumCertificate) -> Result<Self, Self::Error> {
        // Ok(Self::new(
        //     HotStuffMessageType::try_from(u8::try_from(value.message_type).unwrap())?,
        //     ViewId(value.view_number),
        //     TreeNodeHash::try_from(value.node_hash).map_err(|err| err.to_string())?,
        //     value.signature.map(|s| s.try_into()).transpose()?,
        // ))
        todo!()
    }
}

impl TryFrom<proto::consensus::HotStuffTreeNode> for HotStuffTreeNode<CommsPublicKey> {
    type Error = anyhow::Error;

    fn try_from(_value: proto::consensus::HotStuffTreeNode) -> Result<Self, Self::Error> {
        todo!()
        // if value.parent.is_empty() {
        //     return Err("parent not provided".to_string());
        // }
        // let state_root = value
        //     .state_root
        //     .try_into()
        //     .map(StateRoot::new)
        //     .map_err(|_| "Incorrect length for state_root")?;
        // Ok(Self::new(
        //     TreeNodeHash::try_from(value.parent).map_err(|_| "Incorrect length for parent")?,
        //     value
        //         .payload
        //         .map(|p| p.try_into())
        //         .transpose()?
        //         .ok_or("payload not provided")?,
        //     state_root,
        //     value.height,
        // ))
    }
}

impl TryFrom<proto::consensus::ValidatorSignature> for ValidatorSignature {
    type Error = anyhow::Error;

    fn try_from(_value: proto::consensus::ValidatorSignature) -> Result<Self, Self::Error> {
        todo!()
        // Ok(Self {})
    }
}

impl TryFrom<proto::consensus::TariDanPayload> for TariDanPayload {
    type Error = anyhow::Error;

    fn try_from(_value: proto::consensus::TariDanPayload) -> Result<Self, Self::Error> {
        // let instruction_set = value
        //     .instruction_set
        //     .ok_or_else(|| "Instructions were not present".to_string())?
        //     .try_into()?;
        // let checkpoint = value.checkpoint.map(|c| c.try_into()).transpose()?;
        //
        // Ok(Self::new(instruction_set, checkpoint))
        todo!()
    }
}

impl From<Node> for proto::common::Node {
    fn from(node: Node) -> Self {
        Self {
            hash: node.hash().as_bytes().to_vec(),
            parent: node.parent().as_bytes().to_vec(),
            height: node.height(),
            is_committed: node.is_committed(),
        }
    }
}

impl TryFrom<proto::common::Node> for Node {
    type Error = anyhow::Error;

    fn try_from(node: proto::common::Node) -> Result<Self, Self::Error> {
        let hash = TreeNodeHash::try_from(node.hash)?;
        let parent = TreeNodeHash::try_from(node.parent)?;
        let height = node.height;
        let is_committed = node.is_committed;

        Ok(Self::new(hash, parent, height, is_committed))
    }
}

impl From<KeyValue> for proto::validator_node::KeyValue {
    fn from(kv: KeyValue) -> Self {
        Self {
            key: kv.key,
            value: kv.value,
        }
    }
}

impl TryFrom<proto::validator_node::KeyValue> for KeyValue {
    type Error = anyhow::Error;

    fn try_from(kv: proto::validator_node::KeyValue) -> Result<Self, Self::Error> {
        if kv.key.is_empty() {
            return Err(anyhow!("KeyValue: key cannot be empty"));
        }

        Ok(Self {
            key: kv.key,
            value: kv.value,
        })
    }
}

impl From<StateOpLogEntry> for proto::validator_node::StateOpLog {
    fn from(entry: StateOpLogEntry) -> Self {
        let DbStateOpLogEntry {
            height,
            merkle_root,
            operation,
            schema,
            key,
            value,
        } = entry.into_inner();
        Self {
            height,
            merkle_root: merkle_root.map(|r| r.as_bytes().to_vec()).unwrap_or_default(),
            operation: operation.as_op_str().to_string(),
            schema,
            key,
            value: value.unwrap_or_default(),
        }
    }
}
impl TryFrom<proto::validator_node::StateOpLog> for StateOpLogEntry {
    type Error = anyhow::Error;

    fn try_from(value: proto::validator_node::StateOpLog) -> Result<Self, Self::Error> {
        Ok(DbStateOpLogEntry {
            height: value.height,
            merkle_root: Some(value.merkle_root)
                .filter(|r| !r.is_empty())
                .map(TryInto::try_into)
                .transpose()
                .map_err(|_| anyhow!("Invalid merkle root value"))?,
            operation: value
                .operation
                .parse()
                .map_err(|_| anyhow!("Invalid oplog operation string"))?,
            schema: value.schema,
            key: value.key,
            value: Some(value.value).filter(|v| !v.is_empty()),
        }
        .into())
    }
}

//---------------------------------- Signature --------------------------------------------//
impl TryFrom<proto::common::Signature> for Signature {
    type Error = anyhow::Error;

    fn try_from(sig: proto::common::Signature) -> Result<Self, Self::Error> {
        let public_nonce = PublicKey::from_bytes(&sig.public_nonce)?;
        let signature = PrivateKey::from_bytes(&sig.signature)?;

        Ok(Self::new(public_nonce, signature))
    }
}

impl<T: Borrow<Signature>> From<T> for proto::common::Signature {
    fn from(sig: T) -> Self {
        Self {
            public_nonce: sig.borrow().get_public_nonce().to_vec(),
            signature: sig.borrow().get_signature().to_vec(),
        }
    }
}

//---------------------------------- Transaction --------------------------------------------//
impl TryFrom<proto::common::Transaction> for Transaction {
    type Error = anyhow::Error;

    fn try_from(request: proto::common::Transaction) -> Result<Self, Self::Error> {
        let instructions = request
            .instructions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<tari_engine_types::instruction::Instruction>, _>>()?;
        let signature: Signature = request
            .signature
            .ok_or_else(|| anyhow!("invalid signature"))?
            .try_into()?;
        let instruction_signature = signature.try_into().map_err(|s| anyhow!("{}", s))?;
        let sender_public_key =
            PublicKey::from_bytes(&request.sender_public_key).map_err(|_| anyhow!("invalid sender_public_key"))?;
        let transaction = Transaction::new(
            // TODO
            0,
            instructions,
            instruction_signature,
            sender_public_key,
            // TODO
            TransactionMeta::default(),
        );

        Ok(transaction)
    }
}

impl TryFrom<proto::common::Instruction> for tari_engine_types::instruction::Instruction {
    type Error = anyhow::Error;

    fn try_from(request: proto::common::Instruction) -> Result<Self, Self::Error> {
        let template_address =
            Hash::deserialize(&mut &request.template_address[..]).map_err(|_| anyhow!("invalid package_addresss"))?;
        let args = request
            .args
            .into_iter()
            .map(|a| a.try_into())
            .collect::<Result<_, _>>()?;
        let instruction = match request.instruction_type {
            // function
            0 => {
                let function = request.function;
                tari_engine_types::instruction::Instruction::CallFunction {
                    template_address,
                    function,
                    args,
                }
            },
            // method
            1 => {
                let component_address = Hash::deserialize(&mut &request.component_address[..])
                    .map_err(|_| anyhow!("invalid component_address"))?;
                let method = request.method;
                tari_engine_types::instruction::Instruction::CallMethod {
                    template_address,
                    component_address,
                    method,
                    args,
                }
            },
            // 2 => tari_dan_engine::instruction::Instruction::PutLastInstructionOutputOnWorkspace { key: request.key },
            _ => return Err(anyhow!("invalid instruction_type")),
        };

        Ok(instruction)
    }
}

impl From<Transaction> for proto::common::Transaction {
    fn from(transaction: Transaction) -> Self {
        let (instructions, signature, sender_public_key) = transaction.destruct();

        proto::common::Transaction {
            instructions: instructions.into_iter().map(Into::into).collect(),
            signature: Some(signature.signature().into()),
            sender_public_key: sender_public_key.to_vec(),
            // balance_proof: todo!(),
            // inputs: todo!(),
            // max_instruction_outputs: todo!(),
            // outputs: todo!(),
            // fee: todo!(),
            ..Default::default()
        }
    }
}

impl From<tari_engine_types::instruction::Instruction> for proto::common::Instruction {
    fn from(instruction: tari_engine_types::instruction::Instruction) -> Self {
        let mut result = proto::common::Instruction::default();

        match instruction {
            tari_engine_types::instruction::Instruction::CallFunction {
                template_address,
                function,
                args,
            } => {
                result.instruction_type = 0;
                result.template_address = template_address.to_vec();
                result.function = function;
                result.args = args.into_iter().map(|a| a.into()).collect();
            },
            tari_engine_types::instruction::Instruction::CallMethod {
                template_address,
                component_address,
                method,
                args,
            } => {
                result.instruction_type = 1;
                result.template_address = template_address.to_vec();
                result.component_address = component_address.to_vec();
                result.method = method;
                result.args = args.into_iter().map(|a| a.into()).collect();
            },
            _ => todo!(),
        }
        result
    }
}

impl TryFrom<proto::common::Arg> for Arg {
    type Error = anyhow::Error;

    fn try_from(request: proto::common::Arg) -> Result<Self, Self::Error> {
        let data = request.data.clone();
        let arg = match request.arg_type {
            0 => Arg::Literal(data),
            1 => Arg::FromWorkspace(data),
            _ => return Err(anyhow!("invalid arg_type")),
        };

        Ok(arg)
    }
}

impl From<Arg> for proto::common::Arg {
    fn from(arg: Arg) -> Self {
        let mut result = proto::common::Arg::default();

        match arg {
            Arg::Literal(data) => {
                result.arg_type = 0;
                result.data = data;
            },
            Arg::FromWorkspace(data) => {
                result.arg_type = 1;
                result.data = data;
            },
        }

        result
    }
}
