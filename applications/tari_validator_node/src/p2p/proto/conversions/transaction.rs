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
use tari_common_types::types::{PublicKey, Signature};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{ObjectClaim, ShardId, SubstateChange};
use tari_dan_core::message::MempoolMessage;
use tari_dan_engine::transaction::{Transaction, TransactionMeta};
use tari_template_lib::{args::Arg, Hash};

use crate::p2p::proto;

//---------------------------------- Transaction --------------------------------------------//
impl TryFrom<proto::transaction::Transaction> for Transaction {
    type Error = anyhow::Error;

    fn try_from(request: proto::transaction::Transaction) -> Result<Self, Self::Error> {
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
        let meta = request.meta.map(TryInto::try_into).transpose()?;
        let transaction = Transaction::new(
            request.fee,
            instructions,
            instruction_signature,
            sender_public_key,
            meta.unwrap_or_default(),
        );

        Ok(transaction)
    }
}

impl From<Transaction> for proto::transaction::Transaction {
    fn from(transaction: Transaction) -> Self {
        let fee = transaction.fee();
        let meta = transaction.meta().clone();
        let (instructions, signature, sender_public_key) = transaction.destruct();

        proto::transaction::Transaction {
            instructions: instructions.into_iter().map(Into::into).collect(),
            signature: Some(signature.signature().into()),
            sender_public_key: sender_public_key.to_vec(),
            fee,
            meta: Some(meta.into()),
            // balance_proof: todo!(),
            // inputs: todo!(),
            // max_instruction_outputs: todo!(),
            // outputs: todo!(),
            ..Default::default()
        }
    }
}

// -------------------------------- Instruction -------------------------------- //

impl TryFrom<proto::transaction::Instruction> for tari_engine_types::instruction::Instruction {
    type Error = anyhow::Error;

    fn try_from(request: proto::transaction::Instruction) -> Result<Self, Self::Error> {
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

impl From<tari_engine_types::instruction::Instruction> for proto::transaction::Instruction {
    fn from(instruction: tari_engine_types::instruction::Instruction) -> Self {
        let mut result = proto::transaction::Instruction::default();

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

// -------------------------------- Arg -------------------------------- //

impl TryFrom<proto::transaction::Arg> for Arg {
    type Error = anyhow::Error;

    fn try_from(request: proto::transaction::Arg) -> Result<Self, Self::Error> {
        let data = request.data.clone();
        let arg = match request.arg_type {
            0 => Arg::Literal(data),
            1 => Arg::FromWorkspace(data),
            _ => return Err(anyhow!("invalid arg_type")),
        };

        Ok(arg)
    }
}

impl From<Arg> for proto::transaction::Arg {
    fn from(arg: Arg) -> Self {
        let mut result = proto::transaction::Arg::default();

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

// -------------------------------- TransactionMeta -------------------------------- //

impl TryFrom<proto::transaction::TransactionMeta> for TransactionMeta {
    type Error = anyhow::Error;

    fn try_from(val: proto::transaction::TransactionMeta) -> Result<Self, Self::Error> {
        if val.involved_shard_ids.len() != val.involved_substates.len() {
            return Err(anyhow!(
                "involved_shard_ids and involved_shard_ids must have the same length"
            ));
        }

        Ok(TransactionMeta::new(
            val.involved_shard_ids
                .into_iter()
                .map(|s| ShardId::try_from(s).map_err(|e| anyhow!("{}", e)))
                .zip(val.involved_substates.into_iter().map(|c| {
                    proto::transaction::SubstateChange::from_i32(c.change)
                        .ok_or_else(|| anyhow!("invalid change"))
                        .and_then(SubstateChange::try_from)
                }))
                .map(|(a, b)| {
                    let a = a?;
                    let b = b?;
                    Result::<_, anyhow::Error>::Ok((a, (b, ObjectClaim {})))
                })
                .collect::<Result<_, _>>()?,
        ))
    }
}

impl<T: Borrow<TransactionMeta>> From<T> for proto::transaction::TransactionMeta {
    fn from(val: T) -> Self {
        let mut meta = proto::transaction::TransactionMeta::default();
        for (k, (ch, _)) in val.borrow().involved_objects_iter() {
            meta.involved_shard_ids.push(k.as_bytes().to_vec());
            meta.involved_substates.push(proto::transaction::SubstateRef {
                change: proto::transaction::SubstateChange::from(*ch) as i32,
            });
        }
        meta
    }
}

// -------------------------------- SubstateChange -------------------------------- //

impl TryFrom<proto::transaction::SubstateChange> for SubstateChange {
    type Error = anyhow::Error;

    fn try_from(val: proto::transaction::SubstateChange) -> Result<Self, Self::Error> {
        match val {
            proto::transaction::SubstateChange::Create => Ok(SubstateChange::Create),
            proto::transaction::SubstateChange::Exists => Ok(SubstateChange::Exists),
            proto::transaction::SubstateChange::Destroy => Ok(SubstateChange::Destroy),
        }
    }
}

impl From<SubstateChange> for proto::transaction::SubstateChange {
    fn from(val: SubstateChange) -> Self {
        match val {
            SubstateChange::Create => proto::transaction::SubstateChange::Create,
            SubstateChange::Exists => proto::transaction::SubstateChange::Exists,
            SubstateChange::Destroy => proto::transaction::SubstateChange::Destroy,
        }
    }
}

// -------------------------------- MempoolMessage ----------------------------- //

impl TryFrom<proto::transaction::MempoolMessage> for MempoolMessage {
    type Error = anyhow::Error;

    fn try_from(val: proto::transaction::MempoolMessage) -> Result<Self, Self::Error> {
        match val.mempool_message_type {
            0 => Ok(Self::SubmitTransaction(Box::new(Transaction::try_from(
                val.submit_transaction
                    .ok_or_else(|| anyhow!("invalid transaction to submit"))?,
            )?))),
            1 => Ok(Self::RemoveTransaction {
                transaction_hash: Hash::try_from(val.transaction_hash)?,
            }),
            _ => return Err(anyhow!("invalid mempool message type")),
        }
    }
}

impl From<MempoolMessage> for proto::transaction::MempoolMessage {
    fn from(val: MempoolMessage) -> Self {
        match val {
            MempoolMessage::SubmitTransaction(tx) => proto::transaction::MempoolMessage {
                mempool_message_type: 0,
                submit_transaction: Some(proto::transaction::Transaction::from(*tx)),
                transaction_hash: vec![],
            },
            MempoolMessage::RemoveTransaction { transaction_hash } => proto::transaction::MempoolMessage {
                mempool_message_type: 1,
                submit_transaction: None,
                transaction_hash: Vec::from(transaction_hash.as_ref()),
            },
        }
    }
}
