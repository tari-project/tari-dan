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

use std::convert::{TryFrom, TryInto};

use anyhow::anyhow;
use tari_bor::decode_exact;
use tari_common_types::types::{Commitment, PrivateKey, PublicKey};
use tari_crypto::{ristretto::RistrettoComSig, tari_utilities::ByteArray};
use tari_dan_common_types::Epoch;
use tari_dan_p2p::NewTransactionMessage;
use tari_engine_types::{
    confidential::{ConfidentialClaim, ConfidentialOutput},
    instruction::Instruction,
    substate::SubstateAddress,
};
use tari_template_lib::{
    args::Arg,
    crypto::{BalanceProofSignature, RistrettoPublicKeyBytes},
    models::{ConfidentialOutputProof, ConfidentialStatement, ConfidentialWithdrawProof, EncryptedData},
    Hash,
};
use tari_transaction::{SubstateRequirement, Transaction};

use crate::{
    proto::{
        self,
        transaction::{instruction::InstructionType, OptionalVersion},
    },
    utils::checked_copy_fixed,
};

// -------------------------------- NewTransactionMessage -------------------------------- //

impl From<NewTransactionMessage> for proto::transaction::NewTransactionMessage {
    fn from(msg: NewTransactionMessage) -> Self {
        Self {
            transaction: Some((&msg.transaction).into()),
            output_shards: msg.output_shards.into_iter().map(|s| s.as_bytes().to_vec()).collect(),
        }
    }
}

impl TryFrom<proto::transaction::NewTransactionMessage> for NewTransactionMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::transaction::NewTransactionMessage) -> Result<Self, Self::Error> {
        Ok(NewTransactionMessage {
            transaction: value
                .transaction
                .ok_or_else(|| anyhow!("Transaction not provided"))?
                .try_into()?,
            output_shards: value
                .output_shards
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}

//---------------------------------- Transaction --------------------------------------------//
impl TryFrom<proto::transaction::Transaction> for Transaction {
    type Error = anyhow::Error;

    fn try_from(request: proto::transaction::Transaction) -> Result<Self, Self::Error> {
        let instructions = request
            .instructions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;
        let fee_instructions = request
            .fee_instructions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;
        let signature = request
            .signature
            .ok_or_else(|| anyhow!("invalid signature"))?
            .try_into()?;
        let inputs = request
            .inputs
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;
        let input_refs = request
            .input_refs
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;
        let filled_inputs = request
            .filled_inputs
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;
        let min_epoch = request.min_epoch.map(|epoch| Epoch(epoch.epoch));
        let max_epoch = request.max_epoch.map(|epoch| Epoch(epoch.epoch));
        let transaction = Transaction::new(
            fee_instructions,
            instructions,
            signature,
            inputs,
            input_refs,
            filled_inputs,
            min_epoch,
            max_epoch,
        );

        Ok(transaction)
    }
}

impl From<&Transaction> for proto::transaction::Transaction {
    fn from(transaction: &Transaction) -> Self {
        let signature = transaction.signature().clone().into();
        let inputs = transaction.inputs().iter().map(|s| s.as_bytes().to_vec()).collect();
        let input_refs = transaction.input_refs().iter().map(|s| s.as_bytes().to_vec()).collect();
        let filled_inputs = transaction
            .filled_inputs()
            .iter()
            .map(|s| s.as_bytes().to_vec())
            .collect();
        let fee_instructions = transaction.fee_instructions().to_vec();
        let instructions = transaction.instructions().to_vec();
        let min_epoch = transaction
            .min_epoch()
            .map(|epoch| proto::common::Epoch { epoch: epoch.0 });
        let max_epoch = transaction
            .max_epoch()
            .map(|epoch| proto::common::Epoch { epoch: epoch.0 });
        let fee_instructions = fee_instructions.into_iter().map(Into::into).collect();
        let instructions = instructions.into_iter().map(Into::into).collect();
        proto::transaction::Transaction {
            fee_instructions,
            instructions,
            signature: Some(signature),
            inputs,
            input_refs,
            filled_inputs,
            min_epoch,
            max_epoch,
        }
    }
}
// -------------------------------- Instruction -------------------------------- //

impl TryFrom<proto::transaction::Instruction> for Instruction {
    type Error = anyhow::Error;

    fn try_from(request: proto::transaction::Instruction) -> Result<Self, Self::Error> {
        let args = request
            .args
            .into_iter()
            .map(|a| a.try_into())
            .collect::<Result<_, _>>()?;
        let instruction_type =
            InstructionType::from_i32(request.instruction_type).ok_or_else(|| anyhow!("invalid instruction_type"))?;
        let instruction = match instruction_type {
            InstructionType::Function => {
                let function = request.function;
                Instruction::CallFunction {
                    template_address: request.template_address.try_into()?,
                    function,
                    args,
                }
            },
            InstructionType::Method => {
                let method = request.method;
                let component_address = Hash::try_from(request.component_address)?.into();
                Instruction::CallMethod {
                    component_address,
                    method,
                    args,
                }
            },
            InstructionType::PutOutputInWorkspace => {
                Instruction::PutLastInstructionOutputOnWorkspace { key: request.key }
            },
            InstructionType::EmitLog => Instruction::EmitLog {
                level: request.log_level.parse()?,
                message: request.log_message,
            },
            InstructionType::ClaimBurn => Instruction::ClaimBurn {
                claim: Box::new(ConfidentialClaim {
                    public_key: PublicKey::from_canonical_bytes(&request.claim_burn_public_key)
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
            InstructionType::ClaimValidatorFees => Instruction::ClaimValidatorFees {
                epoch: request.claim_validator_fees_epoch,
                validator_public_key: PublicKey::from_canonical_bytes(
                    &request.claim_validator_fees_validator_public_key,
                )
                .map_err(|e| anyhow!("claim_validator_fees_validator_public_key: {}", e))?,
            },
            InstructionType::DropAllProofsInWorkspace => Instruction::DropAllProofsInWorkspace,
            InstructionType::CreateFreeTestCoins => Instruction::CreateFreeTestCoins {
                revealed_amount: request.create_free_test_coins_amount.try_into()?,
                output: tari_bor::decode(&request.create_free_test_coins_output_blob)?,
            },
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
                result.instruction_type = InstructionType::Function as i32;
                result.template_address = template_address.to_vec();
                result.function = function;
                result.args = args.into_iter().map(|a| a.into()).collect();
            },
            Instruction::CallMethod {
                component_address,
                method,
                args,
            } => {
                result.instruction_type = InstructionType::Method as i32;
                result.component_address = component_address.as_bytes().to_vec();
                result.method = method;
                result.args = args.into_iter().map(|a| a.into()).collect();
            },
            Instruction::PutLastInstructionOutputOnWorkspace { key } => {
                result.instruction_type = InstructionType::PutOutputInWorkspace as i32;
                result.key = key;
            },
            Instruction::EmitLog { level, message } => {
                result.instruction_type = InstructionType::EmitLog as i32;
                result.log_level = level.to_string();
                result.log_message = message;
            },
            Instruction::ClaimBurn { claim } => {
                result.instruction_type = InstructionType::ClaimBurn as i32;
                result.claim_burn_commitment_address = claim.output_address.to_vec();
                result.claim_burn_range_proof = claim.range_proof.to_vec();
                result.claim_burn_proof_of_knowledge = Some(claim.proof_of_knowledge.into());
                result.claim_burn_public_key = claim.public_key.to_vec();
                result.claim_burn_withdraw_proof = claim.withdraw_proof.map(Into::into);
            },
            Instruction::ClaimValidatorFees {
                epoch,
                validator_public_key,
            } => {
                result.instruction_type = InstructionType::ClaimValidatorFees as i32;
                result.claim_validator_fees_epoch = epoch;
                result.claim_validator_fees_validator_public_key = validator_public_key.to_vec();
            },
            Instruction::DropAllProofsInWorkspace => {
                result.instruction_type = InstructionType::DropAllProofsInWorkspace as i32;
            },
            // TODO: debugging feature should not be the default. Perhaps a better way to create faucet coins is to mint
            //       a faucet vault in the genesis state for dev networks and use faucet builtin template to withdraw
            //       funds.
            Instruction::CreateFreeTestCoins {
                revealed_amount: amount,
                output,
            } => {
                result.instruction_type = InstructionType::CreateFreeTestCoins as i32;
                result.create_free_test_coins_amount = amount.value() as u64;
                result.create_free_test_coins_output_blob = output
                    .map(|o| tari_bor::encode(&o).unwrap())
                    .unwrap_or_else(|| tari_bor::encode(&None::<ConfidentialOutput>).unwrap());
            },
        }
        result
    }
}

// -------------------------------- Arg -------------------------------- //

impl TryFrom<proto::transaction::Arg> for Arg {
    type Error = anyhow::Error;

    fn try_from(request: proto::transaction::Arg) -> Result<Self, Self::Error> {
        let data = request.data;
        let arg = match request.arg_type {
            0 => Arg::Literal(decode_exact(&data)?),
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
                result.data = tari_bor::encode(&data).unwrap();
            },
            Arg::Workspace(data) => {
                result.arg_type = 1;
                result.data = data;
            },
        }

        result
    }
}

// -------------------------------- SubstateRequirement -------------------------------- //
impl TryFrom<proto::transaction::SubstateRequirement> for SubstateRequirement {
    type Error = anyhow::Error;

    fn try_from(val: proto::transaction::SubstateRequirement) -> Result<Self, Self::Error> {
        let address = SubstateAddress::from_bytes(&val.address)?;
        let version = val.version.map(|v| v.version);
        let substate_specification = SubstateRequirement::new(address, version);
        Ok(substate_specification)
    }
}

impl From<SubstateRequirement> for proto::transaction::SubstateRequirement {
    fn from(val: SubstateRequirement) -> Self {
        Self {
            address: val.address().to_bytes(),
            version: val.version().map(|v| OptionalVersion { version: v }),
        }
    }
}

impl From<&SubstateRequirement> for proto::transaction::SubstateRequirement {
    fn from(val: &SubstateRequirement) -> Self {
        Self {
            address: val.address().to_bytes(),
            version: val.version().map(|v| OptionalVersion { version: v }),
        }
    }
}

// -------------------------------- CommitmentSignature -------------------------------- //

impl TryFrom<proto::transaction::CommitmentSignature> for RistrettoComSig {
    type Error = anyhow::Error;

    fn try_from(val: proto::transaction::CommitmentSignature) -> Result<Self, Self::Error> {
        let u = PrivateKey::from_canonical_bytes(&val.signature_u).map_err(anyhow::Error::msg)?;
        let v = PrivateKey::from_canonical_bytes(&val.signature_v).map_err(anyhow::Error::msg)?;
        let public_nonce = PublicKey::from_canonical_bytes(&val.public_nonce_commitment).map_err(anyhow::Error::msg)?;

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
        let sender_public_nonce = Some(val.sender_public_nonce)
            .filter(|v| !v.is_empty())
            .map(|v| {
                RistrettoPublicKeyBytes::from_bytes(&v)
                    .map_err(|e| anyhow!("Invalid sender_public_nonce: {}", e.to_error_string()))
            })
            .transpose()?
            .ok_or_else(|| anyhow!("sender_public_nonce is missing"))?;

        Ok(ConfidentialStatement {
            commitment: checked_copy_fixed(&val.commitment)
                .ok_or_else(|| anyhow!("Invalid length of commitment bytes"))?,
            sender_public_nonce,
            encrypted_data: EncryptedData(
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
            sender_public_nonce: val.sender_public_nonce.as_bytes().to_vec(),
            encrypted_value: val.encrypted_data.as_ref().to_vec(),
            minimum_value_promise: val.minimum_value_promise,
            revealed_amount: val
                .revealed_amount
                .as_u64_checked()
                .expect("revealed_amount is negative or too large"),
        }
    }
}
