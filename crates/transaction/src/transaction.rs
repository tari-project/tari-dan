//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, fmt::Display};

use indexmap::IndexSet;
use ootle_network::Network;
use tari_engine_types::{
    confidential::MinotariBurnClaimProof,
    indexed_value::IndexedValueError,
    published_template::PublishedTemplateAddress,
    substate::SubstateId,
};
use tari_ootle_common_types::{
    Epoch,
    SubstateAddress,
    SubstateRequirement,
    SubstateRequirementRef,
    ToSubstateAddress,
    VersionedSubstateId,
    committee::CommitteeInfo,
};
use tari_template_lib_types::{
    ClaimedOutputTombstoneAddress,
    ComponentAddress,
    Hash32,
    TemplateAddress,
    crypto::RistrettoPublicKeyBytes,
};

use crate::{
    Blobs,
    Instruction,
    TransactionIntent,
    TransactionSealSignature,
    TransactionSignature,
    TransactionV1,
    builder::TransactionBuilder,
    transaction_id::TransactionId,
    unsealed::UnsealedTransaction,
    v1::UnsealedTransactionV1,
    weight::TransactionWeight,
};

#[derive(Debug, Clone, borsh::BorshSerialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Transaction {
    #[n(0)]
    V1(#[n(0)] TransactionV1),
}

impl Transaction {
    /// Creates a new transaction builder.
    /// NOTE: The network is set to LocalNet. Be sure to set the correct network using for_network.
    /// NOTE: this method will likely be deprecated in the future
    pub fn builder_localnet(max_epoch: Epoch) -> TransactionBuilder {
        Self::builder(Network::LocalNet, max_epoch)
    }

    pub fn builder<N: Into<u8>>(network: N, max_epoch: Epoch) -> TransactionBuilder {
        TransactionBuilder::new(network, max_epoch)
    }

    pub fn new(transaction: UnsealedTransaction, seal_signature: TransactionSealSignature) -> Self {
        match transaction {
            UnsealedTransaction::V1(tx) => Self::V1(TransactionV1::new(tx, seal_signature)),
        }
    }

    pub fn calculate_id(&self) -> TransactionId {
        match self {
            Self::V1(tx) => tx.calculate_id(),
        }
    }

    /// Id and intent commitment derived from a single pass over the blobs. See
    /// [`TransactionV1::calculate_id_and_intent_commitment`].
    pub fn calculate_id_and_intent_commitment(&self) -> (TransactionId, Hash32) {
        match self {
            Self::V1(tx) => tx.calculate_id_and_intent_commitment(),
        }
    }

    pub fn calculate_transaction_weight(&self) -> TransactionWeight {
        match self {
            Self::V1(tx) => tx.calculate_transaction_weight(),
        }
    }

    /// Bytes this transaction occupies in its canonical CBOR encoding.
    ///
    /// This is the figure the network moves and stores: the p2p `Transaction` message carries the
    /// transaction as a single `bor_encoded` field, and the state store persists the same bytes. It
    /// is a pure function of the transaction, so every node computes the same number and can enforce
    /// a byte limit on it without diverging.
    ///
    /// Computed from the derived `CborLen` rather than by encoding, so nothing is allocated.
    pub fn encoded_size(&self) -> usize {
        // The `Result` is vestigial — `encoded_len` cannot fail for a type that derives `CborLen`.
        tari_bor::encoded_len(self).unwrap_or(0)
    }

    pub fn is_dry_run(&self) -> bool {
        match self {
            Transaction::V1(tx) => tx.is_dry_run(),
        }
    }

    pub fn unsealed_transaction(&self) -> &UnsealedTransactionV1 {
        match self {
            Self::V1(tx) => tx.unsealed_transaction(),
        }
    }

    pub fn network(&self) -> u8 {
        match self {
            Self::V1(tx) => tx.network(),
        }
    }

    pub fn fee_instructions(&self) -> &[Instruction] {
        match self {
            Self::V1(tx) => tx.fee_instructions(),
        }
    }

    pub fn instructions(&self) -> &[Instruction] {
        match self {
            Self::V1(tx) => tx.instructions(),
        }
    }

    pub fn blobs(&self) -> &Blobs {
        match self {
            Self::V1(tx) => tx.blobs(),
        }
    }

    pub fn signatures(&self) -> &[TransactionSignature] {
        match self {
            Self::V1(tx) => tx.signatures(),
        }
    }

    pub fn seal_signature(&self) -> &TransactionSealSignature {
        match self {
            Self::V1(tx) => tx.seal_signature(),
        }
    }

    pub fn is_seal_signer_authorized(&self) -> bool {
        match self {
            Self::V1(tx) => tx.is_seal_signer_authorized(),
        }
    }

    pub fn verify_all_signatures(&self) -> bool {
        match self {
            Self::V1(tx) => tx.verify_all_signatures(),
        }
    }

    /// Returns the public key of the main signer: the authorized seal signer if present, otherwise
    /// the first transaction signer. `None` if the transaction has no signers and the seal signer is
    /// not authorized.
    pub fn main_signer(&self) -> Option<RistrettoPublicKeyBytes> {
        if self.is_seal_signer_authorized() {
            return Some(*self.seal_signature().public_key());
        }
        self.signatures().first().map(|sig| *sig.public_key())
    }

    pub fn inputs(&self) -> &IndexSet<SubstateRequirement> {
        match self {
            Self::V1(tx) => tx.inputs(),
        }
    }

    /// Returns (fee instructions, instructions)
    pub fn into_instruction_parts(self) -> (Vec<Instruction>, Vec<Instruction>) {
        match self {
            Self::V1(tx) => tx.into_unsealed_transaction().into_instruction_parts(),
        }
    }

    /// Returns (fee instructions, main instructions, blobs).
    pub fn into_instructions_and_blobs(self) -> (Vec<Instruction>, Vec<Instruction>, crate::Blobs) {
        match self {
            Self::V1(tx) => {
                let unsealed = tx.into_unsealed_transaction();
                let (unsigned, _signatures) = unsealed.into_parts();
                (unsigned.fee_instructions, unsigned.instructions, unsigned.blobs)
            },
        }
    }

    pub fn all_published_templates_iter(&self) -> impl Iterator<Item = (PublishedTemplateAddress, &[u8])> + '_ {
        match self {
            Self::V1(tx) => tx.all_published_templates_iter(),
        }
    }

    pub fn into_parts(self) -> (UnsealedTransactionV1, TransactionSealSignature) {
        match self {
            Self::V1(tx) => tx.into_parts(),
        }
    }

    pub fn all_inputs_iter(&self) -> impl Iterator<Item = SubstateRequirementRef<'_>> + '_ {
        match self {
            Self::V1(tx) => tx.all_inputs_iter(),
        }
    }

    pub fn involved_substate_addresses_iter(&self) -> impl Iterator<Item = SubstateAddress> + '_ {
        self
            .all_inputs_iter()
            // The version does not affect the shard group
            .map(|i| i.or_zero_version().to_substate_address())
            // We define involvement as either being an input or a known output
            .chain(self.known_output_addresses_iter())
    }

    pub fn claim_burn_outputs_iter(&self) -> impl Iterator<Item = ClaimedOutputTombstoneAddress> + '_ {
        self.claim_burn_iter()
            .map(|c| ClaimedOutputTombstoneAddress::from_commitment(c.commitment))
    }

    pub fn claim_burn_iter(&self) -> impl Iterator<Item = &MinotariBurnClaimProof> + '_ {
        self.instructions()
            .iter()
            .chain(self.fee_instructions())
            .filter_map(|i| i.claim_burn())
    }

    pub fn all_inputs_substate_ids_iter(&self) -> impl Iterator<Item = &SubstateId> + '_ {
        self.inputs().iter().map(|i| i.substate_id())
    }

    /// Returns true if the provided committee is involved in at least one input or known output of this transaction.
    /// A committee may be involved even if this function returns false if and only if it is involved in outputs only.
    pub fn is_involved(&self, committee_info: &CommitteeInfo) -> bool {
        if self.is_global() {
            return true;
        }

        self.involved_substate_addresses_iter()
            .any(|addr| committee_info.includes_substate_address(&addr))
    }

    pub fn known_output_addresses_iter(&self) -> impl Iterator<Item = SubstateAddress> + '_ {
        let tx_substate_address = self.calculate_id().to_substate_address();
        std::iter::once(tx_substate_address).chain(
            self.claim_burn_outputs_iter()
                .map(|c| SubstateAddress::from_object_key(c.as_object_key(), 0)),
        )
    }

    pub fn known_outputs_iter(&self) -> impl Iterator<Item = VersionedSubstateId> + '_ {
        let tx_receipt = self.calculate_id().into_receipt_address();
        std::iter::once(VersionedSubstateId::new(tx_receipt, 0)).chain(
            self.claim_burn_outputs_iter()
                .map(SubstateId::from)
                .map(|s| VersionedSubstateId::new(s, 0)),
        )
    }

    pub fn has_publish_template(&self) -> bool {
        self.instructions()
            .iter()
            .chain(self.fee_instructions())
            .any(|i| matches!(i, Instruction::PublishTemplate { .. }))
    }

    pub fn is_global(&self) -> bool {
        self.has_publish_template()
    }

    pub fn publish_templates_iter(&self) -> impl Iterator<Item = &[u8]> + '_ {
        let blobs = self.unsealed_transaction().unsigned_transaction().blobs();
        self.instructions().iter().filter_map(move |i| match i {
            Instruction::PublishTemplate { binary, .. } => blobs.get(*binary).map(|b| b.as_bytes()),
            _ => None,
        })
    }

    pub fn num_inputs(&self) -> usize {
        self.all_inputs_substate_ids_iter().count()
    }

    /// Validate the blob side of the transaction: every `BlobIndex` is in bounds and every
    /// blob is referenced. See `TransactionV1::validate_blob_references`.
    pub fn validate_blob_references(&self) -> Result<(), crate::v1::BlobValidationError> {
        match self {
            Self::V1(tx) => tx.validate_blob_references(),
        }
    }

    pub fn min_epoch(&self) -> Option<Epoch> {
        match self {
            Self::V1(tx) => tx.min_epoch(),
        }
    }

    pub fn max_epoch(&self) -> Epoch {
        match self {
            Self::V1(tx) => tx.max_epoch(),
        }
    }

    pub const fn schema_version(&self) -> u16 {
        match self {
            Self::V1(tx) => tx.schema_version(),
        }
    }

    pub fn as_referenced_components(&self) -> impl Iterator<Item = &ComponentAddress> + '_ {
        match self {
            Self::V1(tx) => tx.as_referenced_components(),
        }
    }

    /// Returns an iterator that iterates over all the statically referenced template addresses in this transaction.
    /// NOTE: This does not include templates required for component method calls.
    pub fn referenced_templates_iter(&self) -> impl Iterator<Item = &TemplateAddress> + '_ {
        self.instructions()
            .iter()
            .chain(self.fee_instructions())
            .filter_map(|instruction| {
                if let Instruction::CallFunction { address, .. } = instruction {
                    Some(address)
                } else {
                    None
                }
            })
    }

    /// Returns all substates addresses referenced by this transaction
    pub fn to_referenced_substates(&self) -> Result<HashSet<SubstateId>, IndexedValueError> {
        match self {
            Self::V1(tx) => tx.to_referenced_substates(),
        }
    }

    pub fn has_inputs_without_version(&self) -> bool {
        match self {
            Self::V1(tx) => tx.has_inputs_without_version(),
        }
    }
}

impl TransactionIntent for Transaction {
    fn calculate_intent_commitment(&self) -> Hash32 {
        match self {
            Self::V1(tx) => tx.calculate_intent_commitment(),
        }
    }
}

impl From<TransactionV1> for Transaction {
    fn from(tx: TransactionV1) -> Self {
        Self::V1(tx)
    }
}

impl Display for Transaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Transaction::V1(tx) => write!(f, "{tx}"),
        }
    }
}

/// Pruned, archive-only counterpart of `Transaction`. Carries blob commitments instead of
/// blob payloads, so API responses can omit the raw bytes while preserving the `TransactionId`,
/// signature verifiability, and the structural shape the UI needs to display.
///
/// Constructed only via `From<Transaction>` (which derives commitments from the full blobs and
/// drops the payloads) or via deserialization of bytes previously written by the storage layer.
#[derive(Debug, Clone, borsh::BorshSerialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum PrunedTransaction {
    #[n(0)]
    V1(#[n(0)] crate::v1::PrunedTransactionV1),
}

impl PrunedTransaction {
    pub fn calculate_id(&self) -> TransactionId {
        match self {
            Self::V1(tx) => tx.calculate_id(),
        }
    }

    pub fn schema_version(&self) -> u16 {
        match self {
            Self::V1(tx) => tx.schema_version(),
        }
    }

    pub fn fee_instructions(&self) -> &[Instruction] {
        match self {
            Self::V1(tx) => tx.fee_instructions(),
        }
    }

    pub fn instructions(&self) -> &[Instruction] {
        match self {
            Self::V1(tx) => tx.instructions(),
        }
    }

    pub fn blob_hashes(&self) -> &crate::BlobHashes {
        match self {
            Self::V1(tx) => tx.blob_hashes(),
        }
    }

    pub fn signatures(&self) -> &[TransactionSignature] {
        match self {
            Self::V1(tx) => tx.signatures(),
        }
    }

    pub fn seal_signature(&self) -> &TransactionSealSignature {
        match self {
            Self::V1(tx) => tx.seal_signature(),
        }
    }

    pub fn verify_all_signatures(&self) -> bool {
        match self {
            Self::V1(tx) => tx.verify_all_signatures(),
        }
    }

    pub fn network(&self) -> u8 {
        match self {
            Self::V1(tx) => tx.network(),
        }
    }

    pub fn min_epoch(&self) -> Option<Epoch> {
        match self {
            Self::V1(tx) => tx.min_epoch(),
        }
    }

    pub fn max_epoch(&self) -> Epoch {
        match self {
            Self::V1(tx) => tx.max_epoch(),
        }
    }

    pub fn is_seal_signer_authorized(&self) -> bool {
        match self {
            Self::V1(tx) => tx.is_seal_signer_authorized(),
        }
    }

    pub fn is_dry_run(&self) -> bool {
        match self {
            Self::V1(tx) => tx.is_dry_run(),
        }
    }
}

impl TransactionIntent for PrunedTransaction {
    fn calculate_intent_commitment(&self) -> Hash32 {
        match self {
            Self::V1(tx) => tx.calculate_intent_commitment(),
        }
    }
}

impl From<Transaction> for PrunedTransaction {
    fn from(tx: Transaction) -> Self {
        match tx {
            Transaction::V1(tx) => Self::V1(tx.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{
        keys::{PublicKey as _, SecretKey},
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    };
    use tari_ootle_common_types::crypto::create_key_pair;
    use tari_template_lib_types::{TemplateAddress, bytes::Bytes};

    use super::*;
    use crate::{args, call_args};

    fn create_transaction() -> TransactionBuilder {
        Transaction::builder(123u8, Epoch(1))
            .create_account(Default::default())
            .call_method(ComponentAddress::from_array([1; 32]), "method", args![
                1,
                2,
                3,
                "string",
                call_args![1, 2],
                Bytes::from_vec(vec![12; 100])
            ])
            .call_function(TemplateAddress::from_array([1; 32]), "function", args![
                1,
                2,
                3,
                ComponentAddress::from_array([1; 32])
            ])
            .put_last_instruction_output_on_workspace("workspace")
            .publish_template(b"template".to_vec())
            .add_input(SubstateRequirement::versioned(
                SubstateId::Component(ComponentAddress::from_array([1; 32])),
                1,
            ))
    }

    #[test]
    fn it_encodes_and_decodes_without_errors() {
        let (k, _) = create_key_pair();
        let subject = create_transaction().build_and_seal(&k);
        let encoded = tari_bor::encode(&subject).unwrap();
        let _decoded = tari_bor::decode::<Transaction>(&encoded).unwrap();
    }

    #[test]
    fn it_correctly_signs_and_verifies() {
        let secret = RistrettoSecretKey::random(&mut rand::rng());
        let public_key = RistrettoPublicKey::from_secret_key(&secret);
        let subject = create_transaction().build_and_seal(&secret);
        assert!(subject.verify_all_signatures());

        let secret2 = RistrettoSecretKey::random(&mut rand::rng());
        let subject = create_transaction()
            .finish()
            .add_signer(&public_key.to_byte_type(), &secret2)
            .seal(&secret);
        assert!(subject.verify_all_signatures());
    }

    #[test]
    fn seal_signer_is_authorized_by_default() {
        let (k, _) = create_key_pair();
        let subject = create_transaction().build_and_seal(&k);
        assert!(subject.is_seal_signer_authorized());
    }

    #[test]
    fn seal_preserves_explicitly_unauthorized_seal_signer() {
        let (k, _) = create_key_pair();
        let subject = create_transaction()
            .with_seal_signer_authorized(false)
            .build_and_seal(&k);
        assert!(!subject.is_seal_signer_authorized());
        assert!(subject.verify_all_signatures());
    }
}
