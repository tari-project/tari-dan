//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_dan_common_types::{Epoch, SubstateRequirement, VersionedSubstateId};
use tari_engine_types::{
    hashing::{hasher32, EngineHashDomainLabel},
    indexed_value::{IndexedValue, IndexedValueError},
    instruction::Instruction,
    substate::SubstateId,
};
use tari_template_lib::{models::ComponentAddress, Hash};

use crate::{builder::TransactionBuilder, transaction_id::TransactionId, TransactionSignature, UnsignedTransaction};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct Transaction {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    id: TransactionId,
    #[serde(flatten)]
    transaction: UnsignedTransaction,
    signatures: Vec<TransactionSignature>,
    /// Inputs filled by some authority. These are not part of the transaction hash nor the signature
    filled_inputs: IndexSet<VersionedSubstateId>,
}

impl Transaction {
    pub fn builder() -> TransactionBuilder {
        TransactionBuilder::new()
    }

    pub fn new(unsigned_transaction: UnsignedTransaction, signatures: Vec<TransactionSignature>) -> Self {
        let mut tx = Self {
            id: TransactionId::default(),
            transaction: unsigned_transaction,
            filled_inputs: IndexSet::new(),
            signatures,
        };
        tx.id = tx.calculate_hash();
        tx
    }

    pub fn sign(mut self, secret: &RistrettoSecretKey) -> Self {
        let sig = TransactionSignature::sign(secret, &self.transaction);
        self.signatures.push(sig);
        self.id = self.calculate_hash();
        self
    }

    pub fn with_filled_inputs(self, filled_inputs: IndexSet<VersionedSubstateId>) -> Self {
        Self { filled_inputs, ..self }
    }

    fn calculate_hash(&self) -> TransactionId {
        hasher32(EngineHashDomainLabel::Transaction)
            .chain(&self.signatures)
            .chain(&self.transaction)
            .result()
            .into_array()
            .into()
    }

    pub fn id(&self) -> &TransactionId {
        &self.id
    }

    pub fn check_id(&self) -> bool {
        let id = self.calculate_hash();
        id == self.id
    }

    pub fn unsigned_transaction(&self) -> &UnsignedTransaction {
        &self.transaction
    }

    pub fn hash(&self) -> Hash {
        self.id.into_array().into()
    }

    pub fn fee_instructions(&self) -> &[Instruction] {
        &self.transaction.fee_instructions
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.transaction.instructions
    }

    pub fn signatures(&self) -> &[TransactionSignature] {
        &self.signatures
    }

    pub fn verify_all_signatures(&self) -> bool {
        if self.signatures.is_empty() {
            return false;
        }

        self.signatures().iter().all(|sig| sig.verify(&self.transaction))
    }

    pub fn inputs(&self) -> &IndexSet<SubstateRequirement> {
        &self.transaction.inputs
    }

    /// Returns (fee instructions, instructions)
    pub fn into_instructions(self) -> (Vec<Instruction>, Vec<Instruction>) {
        (self.transaction.fee_instructions, self.transaction.instructions)
    }

    pub fn into_parts(
        self,
    ) -> (
        UnsignedTransaction,
        Vec<TransactionSignature>,
        IndexSet<VersionedSubstateId>,
    ) {
        (self.transaction, self.signatures, self.filled_inputs)
    }

    pub fn all_inputs_iter(&self) -> impl Iterator<Item = SubstateRequirement> + '_ {
        self.inputs()
            .iter()
            // Filled inputs override other inputs as they are likely filled with versions
            .filter(|i| self.filled_inputs().iter().all(|fi| fi.substate_id() != i.substate_id()))
            .cloned()
            .chain(self.filled_inputs().iter().cloned().map(Into::into))
    }

    pub fn all_inputs_substate_ids_iter(&self) -> impl Iterator<Item = &SubstateId> + '_ {
        self.inputs()
            .iter()
            // Filled inputs override other inputs as they are likely filled with versions
            .filter(|i| self.filled_inputs().iter().all(|fi| fi.substate_id() != i.substate_id()))
            .map(|i| i.substate_id())
            .chain(self.filled_inputs().iter().map(|fi| fi.substate_id()))
    }

    pub fn num_unique_inputs(&self) -> usize {
        self.all_inputs_substate_ids_iter().count()
    }

    pub fn filled_inputs(&self) -> &IndexSet<VersionedSubstateId> {
        &self.filled_inputs
    }

    pub fn filled_inputs_mut(&mut self) -> &mut IndexSet<VersionedSubstateId> {
        &mut self.filled_inputs
    }

    pub fn fee_claims(&self) -> impl Iterator<Item = (Epoch, PublicKey)> + '_ {
        self.instructions()
            .iter()
            .chain(self.fee_instructions())
            .filter_map(|instruction| {
                if let Instruction::ClaimValidatorFees {
                    epoch,
                    validator_public_key,
                } = instruction
                {
                    Some((Epoch(*epoch), validator_public_key.clone()))
                } else {
                    None
                }
            })
    }

    pub fn min_epoch(&self) -> Option<Epoch> {
        self.transaction.min_epoch
    }

    pub fn max_epoch(&self) -> Option<Epoch> {
        self.transaction.max_epoch
    }

    pub fn as_referenced_components(&self) -> impl Iterator<Item = &ComponentAddress> + '_ {
        self.instructions()
            .iter()
            .chain(self.fee_instructions())
            .filter_map(|instruction| {
                if let Instruction::CallMethod { component_address, .. } = instruction {
                    Some(component_address)
                } else {
                    None
                }
            })
    }

    /// Returns all substates addresses referenced by this transaction
    pub fn to_referenced_substates(&self) -> Result<HashSet<SubstateId>, IndexedValueError> {
        let all_instructions = self.instructions().iter().chain(self.fee_instructions());

        let mut substates = HashSet::new();
        for instruction in all_instructions {
            match instruction {
                Instruction::CallFunction { args, .. } => {
                    for arg in args.iter().filter_map(|a| a.as_literal_bytes()) {
                        let value = IndexedValue::from_raw(arg)?;
                        substates.extend(value.referenced_substates().filter(|id| !id.is_virtual()));
                    }
                },
                Instruction::CallMethod {
                    component_address,
                    args,
                    ..
                } => {
                    substates.insert(SubstateId::Component(*component_address));
                    for arg in args.iter().filter_map(|a| a.as_literal_bytes()) {
                        let value = IndexedValue::from_raw(arg)?;
                        substates.extend(value.referenced_substates().filter(|id| !id.is_virtual()));
                    }
                },
                Instruction::ClaimBurn { claim } => {
                    substates.insert(SubstateId::UnclaimedConfidentialOutput(claim.output_address));
                },
                _ => {},
            }
        }
        Ok(substates)
    }

    pub fn has_inputs_without_version(&self) -> bool {
        self.inputs().iter().any(|i| i.version().is_none())
    }
}
