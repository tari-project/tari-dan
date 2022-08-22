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
use tari_core::transactions::transaction_components::SignerSignature;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::TemplateId;
use tari_dan_core::models::{
    CheckpointData,
    HotStuffMessage,
    HotStuffMessageType,
    HotStuffTreeNode,
    InstructionSet,
    Node,
    QuorumCertificate,
    SideChainBlock,
    TariDanPayload,
    TreeNodeHash,
    ValidatorSignature,
    ViewId,
};
use tari_dan_engine::{
    instruction::Transaction,
    instructions::Instruction,
    state::{
        models::{KeyValue, StateOpLogEntry, StateRoot},
        DbStateOpLogEntry,
    },
};
use tari_template_lib::Hash;

use crate::p2p::proto;

impl From<HotStuffMessage<TariDanPayload>> for proto::consensus::HotStuffMessage {
    fn from(source: HotStuffMessage<TariDanPayload>) -> Self {
        Self {
            message_type: i32::from(source.message_type().as_u8()),
            node: source.node().map(|n| n.clone().into()),
            justify: source.justify().map(|j| j.clone().into()),
            partial_sig: source.partial_sig().map(|s| s.clone().into()),
            view_number: source.view_number().as_u64(),
            node_hash: source
                .node_hash()
                .copied()
                .unwrap_or_else(TreeNodeHash::zero)
                .as_bytes()
                .to_vec(),
            checkpoint_signature: source.checkpoint_signature().map(Into::into),
            contract_id: source.contract_id().to_vec(),
        }
    }
}

impl From<HotStuffTreeNode<TariDanPayload>> for proto::consensus::HotStuffTreeNode {
    fn from(source: HotStuffTreeNode<TariDanPayload>) -> Self {
        Self {
            parent: Vec::from(source.parent().as_bytes()),
            payload: Some(source.payload().clone().into()),
            height: source.height(),
            state_root: Vec::from(source.state_root().as_bytes()),
        }
    }
}

impl From<QuorumCertificate> for proto::consensus::QuorumCertificate {
    fn from(source: QuorumCertificate) -> Self {
        Self {
            message_type: i32::from(source.message_type().as_u8()),
            node_hash: Vec::from(source.node_hash().as_bytes()),
            view_number: source.view_number().as_u64(),
            signature: source.signature().map(|s| s.clone().into()),
        }
    }
}

impl From<ValidatorSignature> for proto::consensus::ValidatorSignature {
    fn from(_s: ValidatorSignature) -> Self {
        Self {}
    }
}

impl From<TariDanPayload> for proto::consensus::TariDanPayload {
    fn from(source: TariDanPayload) -> Self {
        let (instruction_set, checkpoint) = source.destruct();
        Self {
            checkpoint: checkpoint.map(|c| c.into()),
            instruction_set: Some(instruction_set.into()),
        }
    }
}

impl From<CheckpointData> for proto::consensus::CheckpointData {
    fn from(_source: CheckpointData) -> Self {
        Self {}
    }
}

impl From<&Instruction> for proto::common::Instruction {
    fn from(source: &Instruction) -> Self {
        Self {
            template_id: source.template_id() as u32,
            method: source.method().to_string(),
            args: Vec::from(source.args()),
            sender: source.sender().to_vec(),
        }
    }
}

impl TryFrom<proto::consensus::HotStuffMessage> for HotStuffMessage<TariDanPayload> {
    type Error = String;

    fn try_from(value: proto::consensus::HotStuffMessage) -> Result<Self, Self::Error> {
        let node_hash = if value.node_hash.is_empty() {
            None
        } else {
            Some(TreeNodeHash::try_from(value.node_hash).map_err(|err| err.to_string())?)
        };
        Ok(Self::new(
            ViewId(value.view_number),
            HotStuffMessageType::try_from(u8::try_from(value.message_type).unwrap())?,
            value.justify.map(|j| j.try_into()).transpose()?,
            value.node.map(|n| n.try_into()).transpose()?,
            node_hash,
            value.partial_sig.map(|p| p.try_into()).transpose()?,
            value.checkpoint_signature.map(|p| p.try_into()).transpose()?,
            value
                .contract_id
                .try_into()
                .map_err(|err| format!("Not a valid contract ID:{}", err))?,
        ))
    }
}

impl TryFrom<proto::consensus::QuorumCertificate> for QuorumCertificate {
    type Error = String;

    fn try_from(value: proto::consensus::QuorumCertificate) -> Result<Self, Self::Error> {
        Ok(Self::new(
            HotStuffMessageType::try_from(u8::try_from(value.message_type).unwrap())?,
            ViewId(value.view_number),
            TreeNodeHash::try_from(value.node_hash).map_err(|err| err.to_string())?,
            value.signature.map(|s| s.try_into()).transpose()?,
        ))
    }
}

impl TryFrom<proto::consensus::HotStuffTreeNode> for HotStuffTreeNode<TariDanPayload> {
    type Error = String;

    fn try_from(value: proto::consensus::HotStuffTreeNode) -> Result<Self, Self::Error> {
        if value.parent.is_empty() {
            return Err("parent not provided".to_string());
        }
        let state_root = value
            .state_root
            .try_into()
            .map(StateRoot::new)
            .map_err(|_| "Incorrect length for state_root")?;
        Ok(Self::new(
            TreeNodeHash::try_from(value.parent).map_err(|_| "Incorrect length for parent")?,
            value
                .payload
                .map(|p| p.try_into())
                .transpose()?
                .ok_or("payload not provided")?,
            state_root,
            value.height,
        ))
    }
}

impl TryFrom<proto::consensus::ValidatorSignature> for ValidatorSignature {
    type Error = String;

    fn try_from(_value: proto::consensus::ValidatorSignature) -> Result<Self, Self::Error> {
        Ok(Self {})
    }
}

impl TryFrom<proto::common::InstructionSet> for InstructionSet {
    type Error = String;

    fn try_from(value: proto::common::InstructionSet) -> Result<Self, Self::Error> {
        let instructions: Vec<Instruction> = value
            .instructions
            .into_iter()
            .map(|i| i.try_into())
            .collect::<Result<_, String>>()?;
        Ok(Self::from_vec(instructions))
    }
}

impl From<InstructionSet> for proto::common::InstructionSet {
    fn from(value: InstructionSet) -> Self {
        Self {
            instructions: value.instructions().iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<proto::common::Instruction> for Instruction {
    type Error = String;

    fn try_from(value: proto::common::Instruction) -> Result<Self, Self::Error> {
        let template_id = TemplateId::try_from(value.template_id)?;
        Ok(Self::new(
            template_id,
            value.method,
            value.args,
            PublicKey::from_bytes(&value.sender).map_err(|e| format!("Invalid public key:{}", e))?,
        ))
    }
}

impl TryFrom<proto::consensus::TariDanPayload> for TariDanPayload {
    type Error = String;

    fn try_from(value: proto::consensus::TariDanPayload) -> Result<Self, Self::Error> {
        let instruction_set = value
            .instruction_set
            .ok_or_else(|| "Instructions were not present".to_string())?
            .try_into()?;
        let checkpoint = value.checkpoint.map(|c| c.try_into()).transpose()?;

        Ok(Self::new(instruction_set, checkpoint))
    }
}

impl TryFrom<proto::consensus::CheckpointData> for CheckpointData {
    type Error = String;

    fn try_from(_value: proto::consensus::CheckpointData) -> Result<Self, Self::Error> {
        Ok(Self::default())
    }
}

impl From<SideChainBlock> for proto::common::SideChainBlock {
    fn from(block: SideChainBlock) -> Self {
        let (node, instructions) = block.destruct();
        Self {
            node: Some(node.into()),
            instructions: Some(instructions.into()),
        }
    }
}

impl TryFrom<proto::common::SideChainBlock> for SideChainBlock {
    type Error = String;

    fn try_from(block: proto::common::SideChainBlock) -> Result<Self, Self::Error> {
        let node = block
            .node
            .map(TryInto::try_into)
            .ok_or_else(|| "No node provided in sidechain block".to_string())??;
        let instructions = block
            .instructions
            .map(TryInto::try_into)
            .ok_or_else(|| "No InstructionSet provided in sidechain block".to_string())??;
        Ok(Self::new(node, instructions))
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

//---------------------------------- SignerSignature --------------------------------------------//
impl<B: Borrow<SignerSignature>> From<B> for proto::common::SignerSignature {
    fn from(signature: B) -> Self {
        Self {
            signer: signature.borrow().signer().to_vec(),
            signature: Some(signature.borrow().signature().into()),
        }
    }
}

impl TryFrom<proto::common::SignerSignature> for SignerSignature {
    type Error = String;

    fn try_from(value: proto::common::SignerSignature) -> Result<Self, Self::Error> {
        Ok(Self::new(
            PublicKey::from_bytes(&value.signer).map_err(|err| err.to_string())?,
            value
                .signature
                .map(TryInto::try_into)
                .ok_or("signature not provided")??,
        ))
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
impl TryFrom<proto::validator_node::SubmitTransactionRequest> for Transaction {
    type Error = String;

    fn try_from(request: proto::validator_node::SubmitTransactionRequest) -> Result<Self, Self::Error> {
        let instructions = request
            .instructions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<tari_dan_engine::instruction::Instruction>, _>>()?;
        let signature: Signature = request.signature.ok_or("invalid signature")?.try_into()?;
        let instruction_signature = signature.try_into()?;
        let sender_public_key =
            PublicKey::from_bytes(&request.sender_public_key).map_err(|_| "invalid sender_public_key")?;
        let transaction = Transaction {
            instructions,
            signature: instruction_signature,
            sender_public_key,
        };

        Ok(transaction)
    }
}

impl TryFrom<proto::validator_node::Instruction> for tari_dan_engine::instruction::Instruction {
    type Error = String;

    fn try_from(request: proto::validator_node::Instruction) -> Result<Self, Self::Error> {
        let package_id = Hash::deserialize(&mut &request.package_id[..]).map_err(|_| "invalid package_id")?;
        let args = request.args.clone();
        let instruction = match request.instruction_type {
            // function
            0 => {
                let template = request.template;
                let function = request.function;
                tari_dan_engine::instruction::Instruction::CallFunction {
                    package_id,
                    template,
                    function,
                    args,
                }
            },
            // method
            1 => {
                let component_id =
                    Hash::deserialize(&mut &request.component_id[..]).map_err(|_| "invalid component_id")?;
                let method = request.method;
                tari_dan_engine::instruction::Instruction::CallMethod {
                    package_id,
                    component_id,
                    method,
                    args,
                }
            },
            _ => return Err("invalid instruction_type".to_string()),
        };

        Ok(instruction)
    }
}

impl From<Transaction> for proto::validator_node::SubmitTransactionRequest {
    fn from(transaction: Transaction) -> Self {
        let instructions = transaction.instructions.into_iter().map(Into::into).collect();
        let signature = transaction.signature.signature();
        let sender_public_key = transaction.sender_public_key.to_vec();

        proto::validator_node::SubmitTransactionRequest {
            instructions,
            signature: Some(signature.into()),
            sender_public_key,
        }
    }
}

impl From<tari_dan_engine::instruction::Instruction> for proto::validator_node::Instruction {
    fn from(instruction: tari_dan_engine::instruction::Instruction) -> Self {
        let mut result = proto::validator_node::Instruction::default();

        match instruction {
            tari_dan_engine::instruction::Instruction::CallFunction {
                package_id,
                template,
                function,
                args,
            } => {
                result.instruction_type = 0;
                result.package_id = package_id.to_vec();
                result.template = template;
                result.function = function;
                result.args = args;
            },
            tari_dan_engine::instruction::Instruction::CallMethod {
                package_id,
                component_id,
                method,
                args,
            } => {
                result.instruction_type = 1;
                result.package_id = package_id.to_vec();
                result.component_id = component_id.to_vec();
                result.method = method;
                result.args = args;
            },
        }

        result
    }
}
