//  Copyright 2023, The Tari Project
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
use tari_common_types::types::{Commitment, PrivateKey, PublicKey, Signature};
use tari_crypto::{ristretto::RistrettoComSig, tari_utilities::ByteArray};
use tari_dan_common_types::ShardId;
use tari_engine_types::{confidential::ConfidentialClaim, instruction::Instruction};
use tari_template_lib::{
    args::Arg,
    crypto::{BalanceProofSignature, RistrettoPublicKeyBytes},
    models::{ConfidentialOutputProof, ConfidentialStatement, ConfidentialWithdrawProof, EncryptedValue},
    Hash,
};
use tari_transaction::{ObjectClaim, SubstateChange, Transaction, TransactionMeta};

use crate::{proto, utils::checked_copy_fixed};

//---------------------------------- Transaction --------------------------------------------//
impl TryFrom<proto::transaction::Transaction> for Transaction {
    type Error = anyhow::Error;

    fn try_from(request: proto::transaction::Transaction) -> Result<Self, Self::Error> {
        let instructions = request
            .instructions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<tari_engine_types::instruction::Instruction>, _>>()?;
        let fee_instructions = request
            .fee_instructions
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
            fee_instructions,
            instructions,
            instruction_signature,
            sender_public_key,
            meta.ok_or_else(|| anyhow!("meta not provided"))?,
        );

        Ok(transaction)
    }
}

impl From<Transaction> for proto::transaction::Transaction {
    fn from(transaction: Transaction) -> Self {
        let meta = transaction.meta().clone();
        let (instructions, fee_instructions, signature, sender_public_key) = transaction.destruct();

        proto::transaction::Transaction {
            fee_instructions: fee_instructions.into_iter().map(Into::into).collect(),
            instructions: instructions.into_iter().map(Into::into).collect(),
            signature: Some(signature.signature().into()),
            sender_public_key: sender_public_key.to_vec(),
            meta: Some(meta.into()),
        }
    }
}

// -------------------------------- Instruction -------------------------------- //

impl TryFrom<proto::transaction::Instruction> for tari_engine_types::instruction::Instruction {
    type Error = anyhow::Error;

    fn try_from(request: proto::transaction::Instruction) -> Result<Self, Self::Error> {
        let args = request
            .args
            .into_iter()
            .map(|a| a.try_into())
            .collect::<Result<_, _>>()?;
        let instruction = match request.instruction_type {
            // function
            0 => {
                let function = request.function;
                Instruction::CallFunction {
                    template_address: request.template_address.try_into()?,
                    function,
                    args,
                }
            },
            // method
            1 => {
                let method = request.method;
                Instruction::CallMethod {
                    component_address: Hash::try_from(request.component_address)?.into(),
                    method,
                    args,
                }
            },
            2 => Instruction::PutLastInstructionOutputOnWorkspace { key: request.key },
            3 => Instruction::EmitLog {
                level: request.log_level.parse()?,
                message: request.log_message,
            },
            4 => Instruction::ClaimBurn {
                claim: Box::new(ConfidentialClaim {
                    public_key: PublicKey::from_bytes(&request.claim_burn_public_key)
                        .map_err(|e| anyhow!("claim_burn_public_key: {}", e))?,
                    output_address: request
                        .claim_burn_commitment_address
                        .as_slice()
                        .try_into()
                        .map_err(|e| anyhow!("claim_burn_commitment_address: {}", e))?,
                    range_proof: request.claim_burn_range_proof,
                    proof_of_knowledge: request
                        .claim_burn_proof_of_knowledge
                        .ok_or_else(|| anyhow!("claim_burn_proof_of_knowledge not provided"))?
                        .try_into()
                        .map_err(|e| anyhow!("claim_burn_proof_of_knowledge: {}", e))?,
                    withdraw_proof: request.claim_burn_withdraw_proof.map(TryInto::try_into).transpose()?,
                }),
            },
            _ => return Err(anyhow!("invalid instruction_type")),
        };

        Ok(instruction)
    }
}

impl From<Instruction> for proto::transaction::Instruction {
    fn from(instruction: Instruction) -> Self {
        let mut result = proto::transaction::Instruction::default();

        match instruction {
            Instruction::CallFunction {
                template_address,
                function,
                args,
            } => {
                result.instruction_type = 0;
                result.template_address = template_address.to_vec();
                result.function = function;
                result.args = args.into_iter().map(|a| a.into()).collect();
            },
            Instruction::CallMethod {
                component_address,
                method,
                args,
            } => {
                result.instruction_type = 1;
                result.component_address = component_address.as_bytes().to_vec();
                result.method = method;
                result.args = args.into_iter().map(|a| a.into()).collect();
            },
            Instruction::PutLastInstructionOutputOnWorkspace { key } => {
                result.instruction_type = 2;
                result.key = key;
            },
            Instruction::EmitLog { level, message } => {
                result.instruction_type = 3;
                result.log_level = level.to_string();
                result.log_message = message;
            },
            Instruction::ClaimBurn { claim } => {
                result.instruction_type = 4;
                result.claim_burn_commitment_address = claim.output_address.to_vec();
                result.claim_burn_range_proof = claim.range_proof.to_vec();
                result.claim_burn_proof_of_knowledge = Some(claim.proof_of_knowledge.into());
                result.claim_burn_public_key = claim.public_key.to_vec();
                result.claim_burn_withdraw_proof = claim.withdraw_proof.map(Into::into);
            },
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
            1 => Arg::Workspace(data),
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
            Arg::Workspace(data) => {
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

        let involved_objects = val
            .involved_shard_ids
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
            .collect::<Result<_, _>>()?;

        Ok(TransactionMeta::new(involved_objects, val.max_outputs))
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
        meta.max_outputs = val.borrow().max_outputs();
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

// -------------------------------- CommitmentSignature -------------------------------- //

impl TryFrom<proto::transaction::CommitmentSignature> for RistrettoComSig {
    type Error = anyhow::Error;

    fn try_from(val: proto::transaction::CommitmentSignature) -> Result<Self, Self::Error> {
        let u = PrivateKey::from_bytes(&val.signature_u)?;
        let v = PrivateKey::from_bytes(&val.signature_v)?;
        let public_nonce = PublicKey::from_bytes(&val.public_nonce_commitment)?;

        Ok(RistrettoComSig::new(Commitment::from_public_key(&public_nonce), u, v))
    }
}

impl From<RistrettoComSig> for proto::transaction::CommitmentSignature {
    fn from(val: RistrettoComSig) -> Self {
        Self {
            public_nonce_commitment: val.public_nonce().to_vec(),
            signature_u: val.u().to_vec(),
            signature_v: val.v().to_vec(),
        }
    }
}
// -------------------------------- ConfidentialWithdrawProof -------------------------------- //

impl TryFrom<proto::transaction::ConfidentialWithdrawProof> for ConfidentialWithdrawProof {
    type Error = anyhow::Error;

    fn try_from(val: proto::transaction::ConfidentialWithdrawProof) -> Result<Self, Self::Error> {
        Ok(ConfidentialWithdrawProof {
            inputs: val
                .inputs
                .into_iter()
                .map(|v| checked_copy_fixed(&v).ok_or_else(|| anyhow!("Invalid length of input commitment bytes")))
                .collect::<Result<_, _>>()?,
            output_proof: val
                .output_proof
                .ok_or_else(|| anyhow!("output_proof is missing"))?
                .try_into()?,
            balance_proof: BalanceProofSignature::from_bytes(&val.balance_proof)
                .map_err(|e| anyhow!("Invalid balance proof signature: {}", e.to_error_string()))?,
        })
    }
}

impl From<ConfidentialWithdrawProof> for proto::transaction::ConfidentialWithdrawProof {
    fn from(val: ConfidentialWithdrawProof) -> Self {
        Self {
            inputs: val.inputs.iter().map(|v| v.to_vec()).collect(),
            output_proof: Some(val.output_proof.into()),
            balance_proof: val.balance_proof.as_bytes().to_vec(),
        }
    }
}

// -------------------------------- ConfidentialOutputProof -------------------------------- //

impl TryFrom<proto::transaction::ConfidentialOutputProof> for ConfidentialOutputProof {
    type Error = anyhow::Error;

    fn try_from(val: proto::transaction::ConfidentialOutputProof) -> Result<Self, Self::Error> {
        Ok(ConfidentialOutputProof {
            output_statement: val
                .output_statement
                .ok_or_else(|| anyhow!("output is missing"))?
                .try_into()?,
            change_statement: val.change_statement.map(TryInto::try_into).transpose()?,
            range_proof: val.range_proof,
        })
    }
}

impl From<ConfidentialOutputProof> for proto::transaction::ConfidentialOutputProof {
    fn from(val: ConfidentialOutputProof) -> Self {
        Self {
            output_statement: Some(val.output_statement.into()),
            change_statement: val.change_statement.map(Into::into),
            range_proof: val.range_proof,
        }
    }
}

// -------------------------------- ConfidentialStatement -------------------------------- //

impl TryFrom<proto::transaction::ConfidentialStatement> for ConfidentialStatement {
    type Error = anyhow::Error;

    fn try_from(val: proto::transaction::ConfidentialStatement) -> Result<Self, Self::Error> {
        Ok(ConfidentialStatement {
            commitment: checked_copy_fixed(&val.commitment)
                .ok_or_else(|| anyhow!("Invalid length of commitment bytes"))?,
            sender_public_nonce: Some(val.sender_public_nonce)
                .filter(|v| !v.is_empty())
                .map(|v| {
                    RistrettoPublicKeyBytes::from_bytes(&v)
                        .map_err(|e| anyhow!("Invalid sender_public_nonce: {}", e.to_error_string()))
                })
                .transpose()?,
            encrypted_value: EncryptedValue(
                checked_copy_fixed(&val.encrypted_value)
                    .ok_or_else(|| anyhow!("Invalid length of encrypted_value bytes"))?,
            ),
            minimum_value_promise: val.minimum_value_promise,
            revealed_amount: val.revealed_amount.try_into()?,
        })
    }
}

impl From<ConfidentialStatement> for proto::transaction::ConfidentialStatement {
    fn from(val: ConfidentialStatement) -> Self {
        Self {
            commitment: val.commitment.to_vec(),
            sender_public_nonce: val
                .sender_public_nonce
                .map(|v| v.as_bytes().to_vec())
                .unwrap_or_default(),
            encrypted_value: val.encrypted_value.as_ref().to_vec(),
            minimum_value_promise: val.minimum_value_promise,
            revealed_amount: val
                .revealed_amount
                .as_u64_checked()
                .expect("revealed_amount is negative or too large"),
        }
    }
}
