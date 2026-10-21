//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Transaction intent commitment.
//!
//! A transaction receipt is bound to its transaction transitively: the receipt is addressed by the
//! [`TransactionId`](crate::TransactionId), and that id is derived from the signing projection
//! *plus* the authorization signatures and the seal signature. Reproducing the id therefore
//! requires the signers' public keys, so linking a transaction to its receipt that way reveals who
//! authorized it.
//!
//! The intent commitment breaks that entanglement. It commits to
//! [`TransactionSignatureFields`] — network, fee instructions, instructions, inputs, epoch bounds
//! and flags — plus the schema version and the per-blob commitments, and nothing else. Whoever
//! kept the transaction can recompute it and compare against the receipt; nobody learns the
//! signers.
//!
//! What it is not:
//!  * **Not a proof of authorship.** No signer material enters the commitment, so two transactions that differ only in
//!    who signed them — identical network, instructions, unversioned inputs, epoch bounds and flags — have identical
//!    commitments and each verifies against the other's receipt. It establishes that a receipt came from *some*
//!    transaction with exactly this intent.
//!  * **Not hiding.** The preimage is the transaction body, which is gossiped in the clear, so anyone who observed the
//!    transaction can recompute the commitment and construct the same comparison. It hides the signers from a party
//!    reading the receipt, nothing more.
//!
//! Invariants:
//!  * The commitment is identical for a `TransactionV1` and the `PrunedTransactionV1` derived from it, matching the
//!    `TransactionId` invariant.
//!  * Blob payloads never enter the commitment — only their per-blob commitments do.

use tari_engine_types::{
    hashing::{EngineHashDomainLabel, hasher32},
    transaction_receipt::TransactionReceipt,
};
use tari_template_lib_types::Hash32;

use crate::{BlobHashes, v1::signature::TransactionSignatureFields};

/// Computes the intent commitment from the signing projection. Shared by the full and pruned
/// forms so both produce the same commitment.
pub(crate) fn calculate_intent_commitment_v1(
    schema_version: u16,
    fields: &TransactionSignatureFields<'_>,
    blob_hashes: &BlobHashes,
) -> Hash32 {
    hasher32(EngineHashDomainLabel::TransactionIntent)
        .chain(&schema_version)
        .chain(fields)
        .chain(blob_hashes)
        .result()
}

/// A transaction form that can be linked to the receipt it produced.
///
/// Implemented for both the full and pruned transaction types; a holder of either can check a
/// receipt without holding the blob payloads.
pub trait TransactionIntent {
    /// The 32-byte commitment to everything the signers authorized.
    fn calculate_intent_commitment(&self) -> Hash32;

    /// Checks `receipt` against this transaction's intent.
    ///
    /// `Ok(())` means the receipt was produced by a transaction with exactly this intent, without
    /// reproducing the transaction id and so without revealing the signers. It does not establish
    /// that *this* transaction produced it — transactions differing only in their signers share a
    /// commitment — nor that the receipt itself is authentic; the caller is responsible for
    /// obtaining the receipt from the chain.
    fn verify_receipt_intent(&self, receipt: &TransactionReceipt) -> Result<(), IntentCommitmentMismatch> {
        let expected = self.calculate_intent_commitment();
        if receipt.intent_commitment == expected {
            Ok(())
        } else {
            Err(IntentCommitmentMismatch {
                expected,
                actual: receipt.intent_commitment,
            })
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("transaction intent commitment mismatch: expected {expected}, receipt has {actual}")]
pub struct IntentCommitmentMismatch {
    pub expected: Hash32,
    pub actual: Hash32,
}

#[cfg(test)]
mod tests {
    use indexmap::IndexSet;
    use ootle_byte_type::ToByteType;
    use tari_crypto::{
        keys::{PublicKey as _, SecretKey},
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    };
    use tari_engine_types::{Epoch, SubstateVersion, substate::SubstateId, transaction_receipt::FinalizeOutcome};
    use tari_ootle_common_types::InputDeclaration;
    use tari_template_lib_types::ComponentAddress;

    use super::*;
    use crate::{
        Blob,
        Blobs,
        Instruction,
        MainSigner,
        PrunedTransactionV1,
        TransactionSealSignature,
        TransactionSignature,
        TransactionV1,
        UnsealedTransactionV1,
        UnsignedTransactionV1,
    };

    fn sample_unsigned() -> UnsignedTransactionV1 {
        let mut inputs = IndexSet::new();
        inputs.insert(InputDeclaration::write_versioned(
            SubstateId::Component(ComponentAddress::from_array([1; 32])),
            SubstateVersion::new(1),
        ));
        UnsignedTransactionV1 {
            network: 42,
            fee_instructions: vec![Instruction::DropAllProofsInWorkspace],
            instructions: vec![
                Instruction::DropAllProofsInWorkspace,
                Instruction::PutLastInstructionOutputOnWorkspace { key: 7 },
            ],
            inputs,
            min_epoch: Some(Epoch(100)),
            max_epoch: Epoch(200),
            main_signer: MainSigner::None,
            dry_run: true,
            blobs: Blobs::from_vec(vec![Blob::from(vec![1, 2, 3])]),
            nonce: 7,
        }
    }

    /// Seals `unsigned` under a fresh random sealer and `num_signers` fresh random signers.
    fn seal(unsigned: UnsignedTransactionV1, num_signers: usize) -> TransactionV1 {
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let seal_signer = RistrettoPublicKey::from_secret_key(&sealer).to_byte_type();
        let sigs = (0..num_signers)
            .map(|_| {
                let sk = RistrettoSecretKey::random(&mut rand::rng());
                TransactionSignature::sign_v1(&sk, &seal_signer, &unsigned)
            })
            .collect();
        let unsealed = UnsealedTransactionV1::new(unsigned, sigs);
        let seal = TransactionSealSignature::sign_v1(&sealer, &unsealed);
        TransactionV1::new(unsealed, seal)
    }

    fn commitment_of(unsigned: UnsignedTransactionV1) -> Hash32 {
        seal(unsigned, 1).calculate_intent_commitment()
    }

    fn receipt_committing_to(commitment: Hash32) -> TransactionReceipt {
        TransactionReceipt {
            outcome: FinalizeOutcome::Commit,
            diff_summary: Default::default(),
            fee_withdrawals: Default::default(),
            events: Default::default(),
            fee_receipt: Default::default(),
            epoch: Epoch(1),
            intent_commitment: commitment,
        }
    }

    #[test]
    fn commitment_is_deterministic() {
        let tx = seal(sample_unsigned(), 1);
        assert_eq!(tx.calculate_intent_commitment(), tx.calculate_intent_commitment());
    }

    /// The pruned form drops blob payloads but must commit to the same intent, so an archive
    /// holder can verify a receipt without the payloads.
    #[test]
    fn commitment_is_stable_across_pruning() {
        for blobs in [
            Blobs::empty(),
            Blobs::from_vec(vec![Blob::from(vec![1, 2, 3]), Blob::from(vec![4, 5])]),
        ] {
            let mut unsigned = sample_unsigned();
            unsigned.blobs = blobs;
            let full = seal(unsigned, 1);
            let expected = full.calculate_intent_commitment();
            let pruned = PrunedTransactionV1::from(full);
            assert_eq!(pruned.calculate_intent_commitment(), expected);
        }
    }

    /// Every field of the committed projection must influence the commitment. A failure here means
    /// a transaction could be altered while still "proving" it produced the same receipt.
    #[test]
    fn commitment_binds_every_projected_field() {
        let base = sample_unsigned();
        let base_commitment = commitment_of(base.clone());

        // network
        let mut u = base.clone();
        u.network = u.network.wrapping_add(1);
        assert_ne!(commitment_of(u), base_commitment, "network");

        // fee_instructions: extra / empty
        let mut u = base.clone();
        u.fee_instructions.push(Instruction::DropAllProofsInWorkspace);
        assert_ne!(commitment_of(u), base_commitment, "fee_instructions (extra)");
        let mut u = base.clone();
        u.fee_instructions.clear();
        assert_ne!(commitment_of(u), base_commitment, "fee_instructions (empty)");

        // instructions: extra / reordered
        let mut u = base.clone();
        u.instructions.push(Instruction::DropAllProofsInWorkspace);
        assert_ne!(commitment_of(u), base_commitment, "instructions (extra)");
        let mut u = base.clone();
        u.instructions.reverse();
        assert_ne!(commitment_of(u), base_commitment, "instructions (reordered)");

        // inputs: extra / version changed
        let mut u = base.clone();
        u.inputs.insert(InputDeclaration::write_versioned(
            SubstateId::Component(ComponentAddress::from_array([9; 32])),
            SubstateVersion::new(1),
        ));
        assert_ne!(commitment_of(u), base_commitment, "inputs (extra)");
        let mut u = base.clone();
        u.inputs = base
            .inputs
            .iter()
            .map(|i| InputDeclaration {
                substate_id: i.substate_id.clone(),
                version: i.version.map(|v| v.next()),
                is_write: i.is_write,
            })
            .collect();
        assert_ne!(commitment_of(u), base_commitment, "inputs (version changed)");
        let mut u = base.clone();
        u.inputs = base
            .inputs
            .iter()
            .map(|i| i.clone().with_is_write(!i.is_write))
            .collect();
        assert_ne!(commitment_of(u), base_commitment, "inputs (intent changed)");

        // min_epoch: value change and Some <-> None; max_epoch: value change
        let mut u = base.clone();
        u.min_epoch = Some(Epoch(101));
        assert_ne!(commitment_of(u), base_commitment, "min_epoch (value)");
        let mut u = base.clone();
        u.min_epoch = None;
        assert_ne!(commitment_of(u), base_commitment, "min_epoch (None)");
        let mut u = base.clone();
        u.max_epoch = Epoch(999);
        assert_ne!(commitment_of(u), base_commitment, "max_epoch (value)");

        // main_signer
        for main_signer in crate::v1::main_signer::other_main_signers(base.main_signer) {
            let mut u = base.clone();
            u.main_signer = main_signer;
            assert_ne!(commitment_of(u), base_commitment, "main_signer {main_signer:?}");
        }

        // dry_run
        let mut u = base.clone();
        u.dry_run = !u.dry_run;
        assert_ne!(commitment_of(u), base_commitment, "dry_run");

        // nonce
        let mut u = base.clone();
        u.nonce = u.nonce.wrapping_add(1);
        assert_ne!(commitment_of(u), base_commitment, "nonce");

        // blobs: an extra payload, and the same count with different bytes
        let mut u = base.clone();
        u.blobs.push(Blob::from(vec![7, 7])).unwrap();
        assert_ne!(commitment_of(u), base_commitment, "blobs (added)");
        let mut u = base.clone();
        u.blobs = Blobs::from_vec(vec![Blob::from(vec![3, 2, 1])]);
        assert_ne!(commitment_of(u), base_commitment, "blobs (contents)");
    }

    /// The point of the commitment: it is independent of who signed, so it never reveals the
    /// signers. The flip side is that transactions differing only in their signers are
    /// indistinguishable by commitment — only the transaction id, which does bind them, differs.
    #[test]
    fn commitment_is_unaffected_by_signatures() {
        let unsigned = sample_unsigned();
        let a = seal(unsigned.clone(), 1);
        let b = seal(unsigned.clone(), 3);

        assert_eq!(a.calculate_intent_commitment(), b.calculate_intent_commitment());
        assert_ne!(a.calculate_id(), b.calculate_id());
    }

    /// The paired derivation exists only to hash the blobs once. It must produce exactly what the
    /// separate entry points do, otherwise an executed transaction's receipt would carry a
    /// different commitment than a verifier computes.
    #[test]
    fn paired_derivation_matches_the_separate_entry_points() {
        let mut unsigned = sample_unsigned();
        unsigned.blobs.push(Blob::from(vec![9; 64])).unwrap();
        let tx = seal(unsigned, 2);

        assert_eq!(
            tx.calculate_id_and_intent_commitment(),
            (tx.calculate_id(), tx.calculate_intent_commitment())
        );
    }

    #[test]
    fn verify_receipt_intent_accepts_matching_receipt() {
        let tx = seal(sample_unsigned(), 2);
        let receipt = receipt_committing_to(tx.calculate_intent_commitment());

        assert_eq!(tx.verify_receipt_intent(&receipt), Ok(()));
        assert_eq!(
            PrunedTransactionV1::from(tx).verify_receipt_intent(&receipt),
            Ok(()),
            "the pruned form checks against the same receipt",
        );
    }

    #[test]
    fn verify_receipt_intent_rejects_another_transactions_receipt() {
        let tx = seal(sample_unsigned(), 1);
        let mut other = sample_unsigned();
        other.instructions.push(Instruction::DropAllProofsInWorkspace);
        let other = seal(other, 1);

        let receipt = receipt_committing_to(other.calculate_intent_commitment());
        let err = tx.verify_receipt_intent(&receipt).unwrap_err();
        assert_eq!(err.expected, tx.calculate_intent_commitment());
        assert_eq!(err.actual, other.calculate_intent_commitment());
    }
}
