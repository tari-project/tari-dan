//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Pruned, archive-only transaction types that retain blob commitments but drop blob payloads.
//!
//! Invariants:
//!  * A `PrunedTransactionV1` derived from a `TransactionV1` produces the same `TransactionId` and the same intent
//!    commitment.
//!  * Signature verification on a pruned form succeeds iff it would have succeeded on the full form.
//!  * There is no public path that lets a caller fabricate a `BlobHashes` — they are obtained only via
//!    `Blobs::hashes()` (i.e. by deriving from real blobs) or via deserialization of a pruned form previously written
//!    by this crate's storage layer.

use indexmap::IndexSet;
use log::*;
use tari_ootle_common_types::{Epoch, InputDeclaration};
use tari_template_lib_types::{Hash32, crypto::RistrettoPublicKeyBytes};

use crate::{
    BlobHashes,
    Blobs,
    Instruction,
    MainSigner,
    TransactionId,
    TransactionIntent,
    TransactionSealSignature,
    TransactionSignature,
    TransactionV1,
    UnsealedTransactionV1,
    UnsignedTransactionV1,
    v1::{
        intent::calculate_intent_commitment_v1,
        signature::{TransactionSignatureFields, verify_sealed_batch},
    },
};

const LOG_TARGET: &str = "tari::ootle::transaction::pruned";

/// Mirror of `UnsignedTransactionV1` but with blob commitments instead of blob payloads.
///
/// All non-blob fields are preserved verbatim so the field projection used in the signing /
/// id digest is byte-identical to the full form.
#[derive(Debug, Clone, borsh::BorshSerialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PrunedUnsignedTransactionV1 {
    #[n(0)]
    pub network: u8,
    #[n(1)]
    #[cbor(decode_with = "crate::v1::unsigned::decode_instructions")]
    pub fee_instructions: Vec<Instruction>,
    #[n(2)]
    #[cbor(decode_with = "crate::v1::unsigned::decode_instructions")]
    pub instructions: Vec<Instruction>,
    #[n(3)]
    #[cbor(with = "tari_bor::adapters::indexset_codec")]
    #[cfg_attr(
        feature = "serde",
        serde(deserialize_with = "tari_ootle_common_types::deserialize_input_declarations")
    )]
    pub inputs: IndexSet<InputDeclaration>,
    #[n(4)]
    pub min_epoch: Option<Epoch>,
    #[n(5)]
    pub max_epoch: Epoch,
    #[n(6)]
    pub main_signer: MainSigner,
    #[n(7)]
    pub dry_run: bool,
    /// Per-blob commitments. Mirrors `UnsignedTransactionV1::blobs.hashes()` of the full form.
    #[n(8)]
    pub blob_hashes: BlobHashes,
    /// Byte size of each blob, parallel to `blob_hashes`. **Not part of the signing/id domain**
    /// — populated at conversion time from `Blobs` so that API consumers and UIs can show
    /// blob sizes without downloading payloads.
    #[n(9)]
    #[borsh(skip)]
    pub blob_sizes: Vec<u32>,
    /// Mirrors `UnsignedTransactionV1::nonce` of the full form.
    #[n(10)]
    pub nonce: u64,
}

impl PrunedUnsignedTransactionV1 {
    pub const fn schema_version(&self) -> u16 {
        1
    }

    pub fn fee_instructions(&self) -> &[Instruction] {
        &self.fee_instructions
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    pub fn inputs(&self) -> &IndexSet<InputDeclaration> {
        &self.inputs
    }

    pub fn blob_hashes(&self) -> &BlobHashes {
        &self.blob_hashes
    }
}

impl From<UnsignedTransactionV1> for PrunedUnsignedTransactionV1 {
    fn from(t: UnsignedTransactionV1) -> Self {
        let blob_hashes = t.blobs.hashes();
        let blob_sizes = t.blobs.iter().map(|b| b.len() as u32).collect();
        Self {
            network: t.network,
            fee_instructions: t.fee_instructions,
            instructions: t.instructions,
            inputs: t.inputs,
            min_epoch: t.min_epoch,
            max_epoch: t.max_epoch,
            main_signer: t.main_signer,
            dry_run: t.dry_run,
            blob_hashes,
            blob_sizes,
            nonce: t.nonce,
        }
    }
}

/// Mirror of `UnsealedTransactionV1` for the pruned form.
#[derive(Debug, Clone, borsh::BorshSerialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PrunedUnsealedTransactionV1 {
    #[n(0)]
    transaction: PrunedUnsignedTransactionV1,
    #[n(1)]
    signatures: Vec<TransactionSignature>,
}

impl PrunedUnsealedTransactionV1 {
    pub const fn schema_version(&self) -> u16 {
        self.transaction.schema_version()
    }

    pub fn unsigned_transaction(&self) -> &PrunedUnsignedTransactionV1 {
        &self.transaction
    }

    pub fn signatures(&self) -> &[TransactionSignature] {
        &self.signatures
    }

    pub fn blob_hashes(&self) -> &BlobHashes {
        &self.transaction.blob_hashes
    }

    pub fn fee_instructions(&self) -> &[Instruction] {
        &self.transaction.fee_instructions
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.transaction.instructions
    }

    pub fn inputs(&self) -> &IndexSet<InputDeclaration> {
        &self.transaction.inputs
    }

    /// Verify all extra signatures against the seal signer.
    pub fn verify_all_signatures(&self, seal_signer: &RistrettoPublicKeyBytes) -> bool {
        if self.signatures.is_empty() {
            return true;
        }
        // Derived once and reused: every signature over this transaction signs the same message, and
        // deriving it hashes the whole body.
        let message =
            TransactionSignature::create_message_v1_pruned(seal_signer, &self.transaction, self.blob_hashes());
        TransactionSignature::verify_all_against_message(&self.signatures, message)
    }
}

impl From<UnsealedTransactionV1> for PrunedUnsealedTransactionV1 {
    fn from(t: UnsealedTransactionV1) -> Self {
        let (transaction, signatures) = t.into_parts();
        Self {
            transaction: PrunedUnsignedTransactionV1::from(transaction),
            signatures,
        }
    }
}

/// Pruned, archive-only counterpart of `TransactionV1`.
///
/// Constructed only via `From<TransactionV1>` (which derives blob commitments from the full
/// blobs and drops the payloads) or via deserialization of bytes previously written by the
/// storage layer.
#[derive(Debug, Clone, borsh::BorshSerialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PrunedTransactionV1 {
    #[n(0)]
    body: PrunedUnsealedTransactionV1,
    #[n(1)]
    seal_signature: TransactionSealSignature,
}

impl TransactionIntent for PrunedTransactionV1 {
    /// Identical to the full form's commitment.
    fn calculate_intent_commitment(&self) -> Hash32 {
        calculate_intent_commitment_v1(
            self.schema_version(),
            &TransactionSignatureFields::from(&self.body.transaction),
            self.blob_hashes(),
        )
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum BlobRehydrationError {
    #[error("blob count mismatch: pruned has {expected}, hydration provided {got}")]
    CountMismatch { expected: usize, got: usize },
    #[error("blob hash mismatch at index {index}")]
    HashMismatch { index: usize },
}

impl PrunedTransactionV1 {
    pub const fn schema_version(&self) -> u16 {
        self.body.schema_version()
    }

    pub fn unsealed_transaction(&self) -> &PrunedUnsealedTransactionV1 {
        &self.body
    }

    pub fn seal_signature(&self) -> &TransactionSealSignature {
        &self.seal_signature
    }

    pub fn signatures(&self) -> &[TransactionSignature] {
        self.body.signatures()
    }

    pub fn blob_hashes(&self) -> &BlobHashes {
        self.body.blob_hashes()
    }

    pub fn fee_instructions(&self) -> &[Instruction] {
        self.body.fee_instructions()
    }

    pub fn instructions(&self) -> &[Instruction] {
        self.body.instructions()
    }

    pub fn inputs(&self) -> &IndexSet<InputDeclaration> {
        self.body.inputs()
    }

    pub fn network(&self) -> u8 {
        self.body.transaction.network
    }

    pub fn min_epoch(&self) -> Option<Epoch> {
        self.body.transaction.min_epoch
    }

    pub fn max_epoch(&self) -> Epoch {
        self.body.transaction.max_epoch
    }

    pub fn declared_main_signer(&self) -> &MainSigner {
        &self.body.transaction.main_signer
    }

    pub fn is_seal_signer_authorized(&self) -> bool {
        self.declared_main_signer().is_seal_signer_authorized()
    }

    pub fn is_dry_run(&self) -> bool {
        self.body.transaction.dry_run
    }

    /// Compute the deterministic transaction id. Identical to the full form's id — see
    /// [`calculate_id_v1`](super::transaction::calculate_id_v1) for the projection invariants.
    pub fn calculate_id(&self) -> TransactionId {
        super::transaction::calculate_id_v1(
            self.schema_version(),
            TransactionSignatureFields::from(&self.body.transaction),
            self.blob_hashes(),
            self.signatures(),
            self.seal_signature.public_key(),
        )
    }

    /// Verify the seal and all extra signatures.
    pub fn verify_all_signatures(&self) -> bool {
        let seal_message = TransactionSealSignature::create_message_v1_pruned(&self.body);
        let authorization_message = TransactionSignature::create_message_v1_pruned(
            self.seal_signature.public_key(),
            &self.body.transaction,
            self.body.blob_hashes(),
        );

        if verify_sealed_batch(
            &self.seal_signature,
            seal_message,
            self.body.signatures(),
            authorization_message,
        ) {
            return true;
        }
        debug!(
            target: LOG_TARGET,
            "Pruned transaction signatures are invalid: the seal or at least one of its {} authorizations does not \
             verify",
            self.body.signatures().len(),
        );
        false
    }

    /// Rehydrate the pruned form back into a full `TransactionV1` by supplying the original
    /// blobs. Verifies that each provided blob's hash matches the stored commitment.
    pub fn rehydrate(self, blobs: Blobs) -> Result<TransactionV1, BlobRehydrationError> {
        let stored = self.blob_hashes().as_slice();
        if stored.len() != blobs.len() {
            return Err(BlobRehydrationError::CountMismatch {
                expected: stored.len(),
                got: blobs.len(),
            });
        }
        for (i, blob) in blobs.iter().enumerate() {
            if blob.hash() != stored[i] {
                return Err(BlobRehydrationError::HashMismatch { index: i });
            }
        }

        let PrunedTransactionV1 { body, seal_signature } = self;
        let PrunedUnsealedTransactionV1 {
            transaction,
            signatures,
        } = body;
        let PrunedUnsignedTransactionV1 {
            network,
            fee_instructions,
            instructions,
            inputs,
            min_epoch,
            max_epoch,
            main_signer,
            dry_run,
            blob_hashes: _,
            blob_sizes: _,
            nonce,
        } = transaction;

        let unsigned = UnsignedTransactionV1 {
            network,
            fee_instructions,
            instructions,
            inputs,
            min_epoch,
            max_epoch,
            main_signer,
            dry_run,
            blobs,
            nonce,
        };
        let unsealed = UnsealedTransactionV1::new(unsigned, signatures);
        Ok(TransactionV1::new(unsealed, seal_signature))
    }
}

impl From<TransactionV1> for PrunedTransactionV1 {
    fn from(t: TransactionV1) -> Self {
        let (unsealed, seal_signature) = t.into_parts();
        Self {
            body: PrunedUnsealedTransactionV1::from(unsealed),
            seal_signature,
        }
    }
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{
        keys::{PublicKey as PublicKeyT, SecretKey},
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    };
    use tari_engine_types::{SubstateVersion, substate::SubstateId};
    use tari_template_lib_types::ComponentAddress;

    use super::*;
    use crate::{Blob, Blobs, TransactionSealSignature, TransactionSignature};

    fn sample_unsigned_with_blobs(blobs: Blobs) -> UnsignedTransactionV1 {
        let mut inputs = IndexSet::new();
        inputs.insert(InputDeclaration::write_versioned(
            SubstateId::Component(ComponentAddress::from_array([1; 32])),
            SubstateVersion::new(1),
        ));
        UnsignedTransactionV1 {
            network: 42,
            fee_instructions: vec![Instruction::DropAllProofsInWorkspace],
            instructions: vec![Instruction::DropAllProofsInWorkspace],
            inputs,
            min_epoch: Some(Epoch(100)),
            max_epoch: Epoch(200),
            main_signer: MainSigner::None,
            dry_run: true,
            blobs,
            nonce: 7,
        }
    }

    fn sealed(unsigned: UnsignedTransactionV1, sealer: &RistrettoSecretKey) -> TransactionV1 {
        let seal_signer = RistrettoPublicKey::from_secret_key(sealer).to_byte_type();
        let extra_sk = RistrettoSecretKey::random(&mut rand::rng());
        let sig = TransactionSignature::sign_v1(&extra_sk, &seal_signer, &unsigned);
        let unsealed = UnsealedTransactionV1::new(unsigned, vec![sig]);
        let seal = TransactionSealSignature::sign_v1(sealer, &unsealed);
        TransactionV1::new(unsealed, seal)
    }

    #[test]
    fn id_is_stable_across_pruning_with_blobs() {
        let blobs = Blobs::from_vec(vec![Blob::from(vec![1, 2, 3]), Blob::from(vec![4, 5, 6, 7, 8])]);
        let unsigned = sample_unsigned_with_blobs(blobs);
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let full = sealed(unsigned, &sealer);
        let id_full = full.calculate_id();
        let pruned = PrunedTransactionV1::from(full);
        let id_pruned = pruned.calculate_id();
        assert_eq!(id_full, id_pruned);
    }

    #[test]
    fn id_is_stable_across_pruning_without_blobs() {
        let unsigned = sample_unsigned_with_blobs(Blobs::empty());
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let full = sealed(unsigned, &sealer);
        let id_full = full.calculate_id();
        let pruned = PrunedTransactionV1::from(full);
        assert_eq!(id_full, pruned.calculate_id());
    }

    #[test]
    fn pruned_form_verifies_signatures_without_blobs() {
        let blobs = Blobs::from_vec(vec![Blob::from(vec![10, 20, 30, 40])]);
        let unsigned = sample_unsigned_with_blobs(blobs);
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let full = sealed(unsigned, &sealer);
        assert!(full.verify_all_signatures());
        let pruned = PrunedTransactionV1::from(full);
        assert!(pruned.verify_all_signatures());
    }

    #[test]
    fn rehydrate_succeeds_with_matching_blobs() {
        let blobs = Blobs::from_vec(vec![Blob::from(vec![1, 2, 3]), Blob::from(vec![4, 5])]);
        let unsigned = sample_unsigned_with_blobs(blobs.clone());
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let full = sealed(unsigned, &sealer);
        let id = full.calculate_id();
        let pruned = PrunedTransactionV1::from(full);
        let rehydrated = pruned.rehydrate(blobs).expect("rehydrate");
        assert_eq!(rehydrated.calculate_id(), id);
        assert!(rehydrated.verify_all_signatures());
    }

    #[test]
    fn rehydrate_rejects_count_mismatch() {
        let blobs = Blobs::from_vec(vec![Blob::from(vec![1])]);
        let unsigned = sample_unsigned_with_blobs(blobs);
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let full = sealed(unsigned, &sealer);
        let pruned = PrunedTransactionV1::from(full);
        let err = pruned.rehydrate(Blobs::empty()).unwrap_err();
        assert!(matches!(err, BlobRehydrationError::CountMismatch { .. }));
    }

    #[test]
    fn rehydrate_rejects_hash_mismatch() {
        let blobs = Blobs::from_vec(vec![Blob::from(vec![1, 2, 3])]);
        let unsigned = sample_unsigned_with_blobs(blobs);
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let full = sealed(unsigned, &sealer);
        let pruned = PrunedTransactionV1::from(full);
        let bad = Blobs::from_vec(vec![Blob::from(vec![9, 9, 9])]);
        let err = pruned.rehydrate(bad).unwrap_err();
        assert_eq!(err, BlobRehydrationError::HashMismatch { index: 0 });
    }

    #[test]
    fn pruned_signature_rejects_tampered_field() {
        let blobs = Blobs::from_vec(vec![Blob::from(vec![1, 2, 3])]);
        let unsigned = sample_unsigned_with_blobs(blobs);
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let full = sealed(unsigned, &sealer);
        let mut pruned = PrunedTransactionV1::from(full);
        // Tamper a field — seal verification must fail.
        pruned.body.transaction.dry_run = !pruned.body.transaction.dry_run;
        assert!(!pruned.verify_all_signatures());
    }

    #[test]
    fn pruned_signature_rejects_tampered_blob_hashes() {
        let blobs = Blobs::from_vec(vec![Blob::from(vec![1, 2, 3])]);
        let unsigned = sample_unsigned_with_blobs(blobs);
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let full = sealed(unsigned, &sealer);
        let mut pruned = PrunedTransactionV1::from(full);
        // Replace the stored blob_hashes with hashes of different data.
        pruned.body.transaction.blob_hashes = Blobs::from_vec(vec![Blob::from(vec![9, 9, 9])]).hashes();
        assert!(!pruned.verify_all_signatures());
    }
}
