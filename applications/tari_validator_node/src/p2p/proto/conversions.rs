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

use borsh::de::BorshDeserialize;
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_comms::types::CommsPublicKey;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::TemplateId;
use tari_dan_core::models::{
    CheckpointData,
    HotStuffMessage,
    HotStuffMessageType,
    HotStuffTreeNode,
    Node,
    QuorumCertificate,
    TariDanPayload,
    TreeNodeHash,
    ValidatorSignature,
    ViewId,
};
use tari_dan_engine::{
    instruction::Transaction,
    state::{
        models::{KeyValue, StateOpLogEntry, StateRoot},
        DbStateOpLogEntry,
    },
};
use tari_template_lib::{args::Arg, Hash};

use crate::p2p::proto;

impl From<HotStuffMessage<TariDanPayload, CommsPublicKey>> for proto::consensus::HotStuffMessage {
    fn from(source: HotStuffMessage<TariDanPayload, CommsPublicKey>) -> Self {
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
    fn from(source: HotStuffTreeNode<CommsPublicKey>) -> Self {
        todo!()
        // Self {
        //     parent: Vec::from(source.parent().as_bytes()),
        //     payload: Some(source.payload().clone().into()),
        //     height: source.height(),
        //     state_root: Vec::from(source.state_root().as_bytes()),
        // }
    }
}

impl From<QuorumCertificate> for proto::consensus::QuorumCertificate {
    fn from(source: QuorumCertificate) -> Self {
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
    type Error = String;

    fn try_from(value: proto::consensus::HotStuffMessage) -> Result<Self, Self::Error> {
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
    type Error = String;

    fn try_from(value: proto::consensus::QuorumCertificate) -> Result<Self, Self::Error> {
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
    type Error = String;

    fn try_from(value: proto::consensus::HotStuffTreeNode) -> Result<Self, Self::Error> {
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
    type Error = String;

    fn try_from(_value: proto::consensus::ValidatorSignature) -> Result<Self, Self::Error> {
        todo!()
        // Ok(Self {})
    }
}

impl TryFrom<proto::consensus::TariDanPayload> for TariDanPayload {
    type Error = String;

    fn try_from(value: proto::consensus::TariDanPayload) -> Result<Self, Self::Error> {
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
    type Error = String;

    fn try_from(node: proto::common::Node) -> Result<Self, Self::Error> {
        let hash = TreeNodeHash::try_from(node.hash).map_err(|err| err.to_string())?;
        let parent = TreeNodeHash::try_from(node.parent).map_err(|err| err.to_string())?;
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
    type Error = String;

    fn try_from(kv: proto::validator_node::KeyValue) -> Result<Self, Self::Error> {
        if kv.key.is_empty() {
            return Err("KeyValue: key cannot be empty".to_string());
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
    type Error = String;

    fn try_from(value: proto::validator_node::StateOpLog) -> Result<Self, Self::Error> {
        Ok(DbStateOpLogEntry {
            height: value.height,
            merkle_root: Some(value.merkle_root)
                .filter(|r| !r.is_empty())
                .map(TryInto::try_into)
                .transpose()
                .map_err(|_| "Invalid merkle root value".to_string())?,
            operation: value
                .operation
                .parse()
                .map_err(|_| "Invalid oplog operation string".to_string())?,
            schema: value.schema,
            key: value.key,
            value: Some(value.value).filter(|v| !v.is_empty()),
        }
        .into())
    }
}

//---------------------------------- Signature --------------------------------------------//
impl TryFrom<proto::common::Signature> for Signature {
    type Error = String;

    fn try_from(sig: proto::common::Signature) -> Result<Self, Self::Error> {
        let public_nonce = PublicKey::from_bytes(&sig.public_nonce).map_err(|e| e.to_string())?;
        let signature = PrivateKey::from_bytes(&sig.signature).map_err(|e| e.to_string())?;

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
    type Error = String;

    fn try_from(request: proto::common::Transaction) -> Result<Self, Self::Error> {
        // let instructions = request
        //     .instructions
        //     .into_iter()
        //     .map(TryInto::try_into)
        //     .collect::<Result<Vec<tari_dan_engine::instruction::Instruction>, _>>()?;
        // let signature: Signature = request.signature.ok_or("invalid signature")?.try_into()?;
        // let instruction_signature = signature.try_into()?;
        // let sender_public_key =
        //     PublicKey::from_bytes(&request.sender_public_key).map_err(|_| "invalid sender_public_key")?;
        // let transaction = Transaction::new(instructions, instruction_signature, sender_public_key);
        //
        // Ok(transaction)
        todo!()
    }
}

impl TryFrom<proto::common::Instruction> for tari_dan_engine::instruction::Instruction {
    type Error = String;

    fn try_from(request: proto::common::Instruction) -> Result<Self, Self::Error> {
        let package_address =
            Hash::deserialize(&mut &request.package_address[..]).map_err(|_| "invalid package_addresss")?;
        let args = request
            .args
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<Arg>, _>>()?;
        let instruction = match request.instruction_type {
            // function
            0 => {
                let template = request.template;
                let function = request.function;
                tari_dan_engine::instruction::Instruction::CallFunction {
                    package_address,
                    template,
                    function,
                    args: todo!(),
                }
            },
            // method
            1 => {
                let component_address =
                    Hash::deserialize(&mut &request.component_address[..]).map_err(|_| "invalid component_address")?;
                let method = request.method;
                tari_dan_engine::instruction::Instruction::CallMethod {
                    package_address,
                    component_address,
                    method,
                    args: todo!(),
                }
            },
            2 => tari_dan_engine::instruction::Instruction::PutLastInstructionOutputOnWorkspace { key: request.key },
            _ => return Err("invalid instruction_type".to_string()),
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
            balance_proof: todo!(),
            inputs: todo!(),
            max_instruction_outputs: todo!(),
            outputs: todo!(),
            fee: todo!(),
        }
    }
}

impl From<tari_dan_engine::instruction::Instruction> for proto::common::Instruction {
    fn from(instruction: tari_dan_engine::instruction::Instruction) -> Self {
        // let mut result = proto::validator_node::Instruction::default();
        //
        // match instruction {
        //     tari_dan_engine::instruction::Instruction::CallFunction {
        //         package_address,
        //         template,
        //         function,
        //         args,
        //     } => {
        //         result.instruction_type = 0;
        //         result.package_address = package_address.to_vec();
        //         result.template = template;
        //         result.function = function;
        //         result.args = args.into_iter().map(|a| a.to_bytes()).collect();
        //     },
        //     tari_dan_engine::instruction::Instruction::CallMethod {
        //         component_address,
        //         package_address,
        //         method,
        //         args,
        //     } => {
        //         result.instruction_type = 1;
        //         result.package_address = package_address.to_vec();
        //         result.component_address = component_address.to_vec();
        //         result.method = method;
        //         result.args = args.into_iter().map(|a| a.to_bytes()).collect();
        //     },
        //     _ => todo!(),
        // }
        //
        // result
        todo!()
    }
}

impl TryFrom<proto::validator_node::Arg> for Arg {
    type Error = String;

    fn try_from(request: proto::validator_node::Arg) -> Result<Self, Self::Error> {
        let data = request.data.clone();
        let arg = match request.arg_type {
            0 => Arg::Literal(data),
            1 => Arg::FromWorkspace(data),
            _ => return Err("invalid arg_type".to_string()),
        };

        Ok(arg)
    }
}

impl From<Arg> for proto::validator_node::Arg {
    fn from(arg: Arg) -> Self {
        let mut result = proto::validator_node::Arg::default();

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
