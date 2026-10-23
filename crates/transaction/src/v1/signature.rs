//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use indexmap::IndexSet;
use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey as PublicKeyT,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities,
    tari_utilities::ByteArray,
};
use tari_ootle_common_types::{
    Epoch,
    InputDeclaration,
    signature::{SignatureOutput, Sr25519SignatureBytes, TRANSACTION_SIGNING_CONTEXT},
};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{
    BlobHashes,
    Instruction,
    MainSigner,
    UnsealedTransactionV1,
    UnsignedTransaction,
    UnsignedTransactionV1,
    hashing::transaction_hasher_v1,
    unsealed::UnsealedTransaction,
    v1::pruned::{PrunedUnsealedTransactionV1, PrunedUnsignedTransactionV1},
};

/// Identifies a field of the canonical signing preimage, in chain order.
///
/// The discriminants are the on-the-wire field tags used to stream a transaction to a hardware
/// signer (e.g. the Ootle Ledger app). The signer reconstructs the exact byte sequence chained
/// into the message digest by concatenating each segment's bytes in ascending field order
/// (`SealSigner` only present for an authorization signature; `Signatures` only for a seal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PreimageField {
    SchemaVersion = 0,
    SealSigner = 1,
    Network = 2,
    FeeInstructions = 3,
    Instructions = 4,
    Inputs = 5,
    MinEpoch = 6,
    MaxEpoch = 7,
    MainSigner = 8,
    DryRun = 9,
    Nonce = 10,
    BlobHashes = 11,
    Signatures = 12,
}

/// One ordered segment of the canonical signing preimage: the exact borsh bytes chained into the
/// message digest for `field`. Concatenating every segment's `bytes` in order yields the precise
/// byte sequence that `create_message_v1` feeds into the domain-separated hasher (after the
/// domain-separation preamble, which the signer prepends itself).
#[derive(Debug, Clone)]
pub struct PreimageSegment {
    pub field: PreimageField,
    pub bytes: Vec<u8>,
}

fn preimage_segment<T: borsh::BorshSerialize + ?Sized>(field: PreimageField, value: &T) -> PreimageSegment {
    PreimageSegment {
        field,
        // BorshSerialize is infallible for these types; the hasher relies on the same guarantee.
        bytes: borsh::to_vec(value).expect("BorshSerialize is infallible for transaction fields"),
    }
}

#[derive(Debug, Clone, Eq, PartialEq, borsh::BorshSerialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TransactionSealSignature {
    #[n(0)]
    public_key: RistrettoPublicKeyBytes,
    #[n(1)]
    signature: Sr25519SignatureBytes,
}

impl TransactionSealSignature {
    pub fn new(public_key: RistrettoPublicKeyBytes, signature: Sr25519SignatureBytes) -> Self {
        Self { public_key, signature }
    }

    pub fn sign_v1(secret_key: &RistrettoSecretKey, transaction: &UnsealedTransactionV1) -> Self {
        let public_key = RistrettoPublicKey::from_secret_key(secret_key);

        let message = Self::create_message_v1(transaction);
        Self {
            signature: TRANSACTION_SIGNING_CONTEXT.sign(secret_key, &message),
            public_key: public_key.to_byte_type(),
        }
    }

    pub fn verify(&self, transaction: &UnsealedTransaction) -> bool {
        match transaction {
            UnsealedTransaction::V1(t) => self.verify_v1(t),
        }
    }

    pub fn verify_v1(&self, transaction: &UnsealedTransactionV1) -> bool {
        self.verify_message(Self::create_message_v1(transaction))
    }

    /// Verifies the seal against a transaction whose blob commitments have already been derived.
    ///
    /// Deriving them hashes every blob payload, so a caller that also verifies the authorization
    /// signatures should derive [`BlobHashes`] once and use this for both.
    pub fn verify_v1_with_blob_hashes(&self, transaction: &UnsealedTransactionV1, blob_hashes: &BlobHashes) -> bool {
        self.verify_message(Self::create_message_v1_with_blob_hashes(transaction, blob_hashes))
    }

    /// Verifies this seal against an already-derived signing message.
    pub fn verify_message(&self, message: [u8; 64]) -> bool {
        TRANSACTION_SIGNING_CONTEXT.verify(&self.signature, &self.public_key, &message)
    }

    pub fn signature(&self) -> &Sr25519SignatureBytes {
        &self.signature
    }

    pub fn public_key(&self) -> &RistrettoPublicKeyBytes {
        &self.public_key
    }

    pub fn to_ristretto_public_key(&self) -> Result<RistrettoPublicKey, tari_utilities::ByteArrayError> {
        RistrettoPublicKey::from_canonical_bytes(self.public_key.as_bytes())
    }

    pub fn create_message(transaction: &UnsealedTransaction) -> [u8; 64] {
        match transaction {
            UnsealedTransaction::V1(t) => Self::create_message_v1(t),
        }
    }

    pub fn create_message_v1(transaction: &UnsealedTransactionV1) -> [u8; 64] {
        let blob_hashes = transaction.unsigned_transaction().blobs.hashes();
        Self::create_message_v1_with_blob_hashes(transaction, &blob_hashes)
    }

    /// Seal message for a transaction whose blob commitments have already been derived. Produces
    /// the same digest as [`Self::create_message_v1`] given commitments over the same blobs.
    pub fn create_message_v1_with_blob_hashes(
        transaction: &UnsealedTransactionV1,
        blob_hashes: &BlobHashes,
    ) -> [u8; 64] {
        Self::create_message_v1_inner(
            transaction.schema_version(),
            TransactionSignatureFields::from(transaction.unsigned_transaction()),
            blob_hashes,
            transaction.signatures(),
        )
    }

    /// The ordered borsh segments chained into the seal ("SealSignature") message digest.
    ///
    /// Single source of truth for streaming a seal signing request to a hardware signer: the
    /// concatenation of the returned segment bytes is byte-identical to what
    /// [`Self::create_message_v1`] feeds into the hasher (after the domain-separation preamble for
    /// label `"SealSignature"`). Keep this in lock-step with `create_message_v1_inner`.
    pub fn signing_preimage_v1(transaction: &UnsealedTransactionV1) -> Vec<PreimageSegment> {
        let unsigned = transaction.unsigned_transaction();
        let blob_hashes = unsigned.blobs.hashes();
        vec![
            preimage_segment(PreimageField::SchemaVersion, &transaction.schema_version()),
            preimage_segment(PreimageField::Network, &unsigned.network),
            preimage_segment(PreimageField::FeeInstructions, unsigned.fee_instructions.as_slice()),
            preimage_segment(PreimageField::Instructions, unsigned.instructions.as_slice()),
            preimage_segment(PreimageField::Inputs, &unsigned.inputs),
            preimage_segment(PreimageField::MinEpoch, &unsigned.min_epoch),
            preimage_segment(PreimageField::MaxEpoch, &unsigned.max_epoch),
            preimage_segment(PreimageField::MainSigner, &unsigned.main_signer),
            preimage_segment(PreimageField::DryRun, &unsigned.dry_run),
            preimage_segment(PreimageField::Nonce, &unsigned.nonce),
            preimage_segment(PreimageField::BlobHashes, &blob_hashes),
            preimage_segment(PreimageField::Signatures, transaction.signatures()),
        ]
    }

    /// Pruned-form seal message. Uses the stored `BlobHashes` and produces the same digest as
    /// the equivalent full form would.
    pub fn create_message_v1_pruned(transaction: &PrunedUnsealedTransactionV1) -> [u8; 64] {
        Self::create_message_v1_inner(
            transaction.schema_version(),
            TransactionSignatureFields::from(transaction.unsigned_transaction()),
            transaction.blob_hashes(),
            transaction.signatures(),
        )
    }

    fn create_message_v1_inner(
        schema_version: u16,
        fields: TransactionSignatureFields<'_>,
        blob_hashes: &BlobHashes,
        signatures: &[TransactionSignature],
    ) -> [u8; 64] {
        // Project explicitly so blob bytes never enter the digest — only their commitments do.
        transaction_hasher_v1("SealSignature")
            .chain(&schema_version)
            .chain(&fields)
            .chain(blob_hashes)
            .chain(signatures)
            .result()
    }

    pub fn verify_v1_pruned(&self, transaction: &PrunedUnsealedTransactionV1) -> bool {
        self.verify_message(Self::create_message_v1_pruned(transaction))
    }
}

impl From<SignatureOutput> for TransactionSealSignature {
    fn from(output: SignatureOutput) -> Self {
        Self {
            public_key: output.public_key.to_byte_type(),
            signature: output.signature,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, borsh::BorshSerialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TransactionSignature {
    #[n(0)]
    public_key: RistrettoPublicKeyBytes,
    #[n(1)]
    signature: Sr25519SignatureBytes,
}

impl TransactionSignature {
    pub fn new(public_key: RistrettoPublicKeyBytes, signature: Sr25519SignatureBytes) -> Self {
        Self { public_key, signature }
    }

    pub fn sign(
        secret_key: &RistrettoSecretKey,
        seal_signer: &RistrettoPublicKeyBytes,
        transaction: &UnsignedTransaction,
    ) -> Self {
        match transaction {
            UnsignedTransaction::V1(v1) => Self::sign_v1(secret_key, seal_signer, v1),
        }
    }

    pub fn sign_v1(
        secret_key: &RistrettoSecretKey,
        seal_signer: &RistrettoPublicKeyBytes,
        transaction: &UnsignedTransactionV1,
    ) -> Self {
        let public_key = RistrettoPublicKey::from_secret_key(secret_key);
        let message = Self::create_message_v1(seal_signer, transaction);

        Self {
            signature: TRANSACTION_SIGNING_CONTEXT.sign(secret_key, &message),
            public_key: public_key.to_byte_type(),
        }
    }

    pub fn verify_v1(&self, seal_signer: &RistrettoPublicKeyBytes, transaction: &UnsignedTransactionV1) -> bool {
        self.verify_message(Self::create_message_v1(seal_signer, transaction))
    }

    /// Verifies this signature against an already-derived signing message.
    ///
    /// The message is identical for every signature over the same transaction, and deriving it
    /// hashes the entire body — including a commitment over every blob's bytes. A caller verifying
    /// more than one signature MUST derive the message once and use this: deriving it per signature
    /// makes verification cost `O(signatures × body bytes)`, which an attacker controls on both
    /// factors within a single transaction.
    pub fn verify_message(&self, message: [u8; 64]) -> bool {
        TRANSACTION_SIGNING_CONTEXT.verify(&self.signature, &self.public_key, &message)
    }

    pub fn signature(&self) -> &Sr25519SignatureBytes {
        &self.signature
    }

    pub fn public_key(&self) -> &RistrettoPublicKeyBytes {
        &self.public_key
    }

    pub fn create_message(seal_signer: &RistrettoPublicKeyBytes, transaction: &UnsignedTransaction) -> [u8; 64] {
        match transaction {
            UnsignedTransaction::V1(v1) => Self::create_message_v1(seal_signer, v1),
        }
    }

    pub fn create_message_v1(seal_signer: &RistrettoPublicKeyBytes, transaction: &UnsignedTransactionV1) -> [u8; 64] {
        let blob_hashes = transaction.blobs.hashes();
        Self::create_message_v1_with_blob_hashes(seal_signer, transaction, &blob_hashes)
    }

    /// Authorization message for a transaction whose blob commitments have already been derived.
    /// Produces the same digest as [`Self::create_message_v1`] given commitments over the same
    /// blobs.
    pub fn create_message_v1_with_blob_hashes(
        seal_signer: &RistrettoPublicKeyBytes,
        transaction: &UnsignedTransactionV1,
        blob_hashes: &BlobHashes,
    ) -> [u8; 64] {
        Self::create_message_v1_inner(
            seal_signer,
            transaction.schema_version(),
            TransactionSignatureFields::from(transaction),
            blob_hashes,
        )
    }

    /// The ordered borsh segments chained into the authorization ("Signature") message digest.
    ///
    /// This is the single source of truth for streaming an authorization signing request to a
    /// hardware signer: the concatenation of the returned segment bytes is byte-identical to what
    /// [`Self::create_message_v1`] feeds into the hasher (after the domain-separation preamble for
    /// label `"Signature"`). Keep this in lock-step with `create_message_v1_inner`.
    pub fn signing_preimage_v1(
        seal_signer: &RistrettoPublicKeyBytes,
        transaction: &UnsignedTransactionV1,
    ) -> Vec<PreimageSegment> {
        let blob_hashes = transaction.blobs.hashes();
        vec![
            preimage_segment(PreimageField::SchemaVersion, &transaction.schema_version()),
            preimage_segment(PreimageField::SealSigner, seal_signer),
            preimage_segment(PreimageField::Network, &transaction.network),
            preimage_segment(PreimageField::FeeInstructions, transaction.fee_instructions.as_slice()),
            preimage_segment(PreimageField::Instructions, transaction.instructions.as_slice()),
            preimage_segment(PreimageField::Inputs, &transaction.inputs),
            preimage_segment(PreimageField::MinEpoch, &transaction.min_epoch),
            preimage_segment(PreimageField::MaxEpoch, &transaction.max_epoch),
            preimage_segment(PreimageField::MainSigner, &transaction.main_signer),
            preimage_segment(PreimageField::DryRun, &transaction.dry_run),
            preimage_segment(PreimageField::Nonce, &transaction.nonce),
            preimage_segment(PreimageField::BlobHashes, &blob_hashes),
        ]
    }

    /// Pruned-form extra-signer message. Uses the stored `BlobHashes`.
    pub fn create_message_v1_pruned(
        seal_signer: &RistrettoPublicKeyBytes,
        transaction: &PrunedUnsignedTransactionV1,
        blob_hashes: &BlobHashes,
    ) -> [u8; 64] {
        Self::create_message_v1_inner(
            seal_signer,
            transaction.schema_version(),
            TransactionSignatureFields::from(transaction),
            blob_hashes,
        )
    }

    fn create_message_v1_inner(
        seal_signer: &RistrettoPublicKeyBytes,
        schema_version: u16,
        fields: TransactionSignatureFields<'_>,
        blob_hashes: &BlobHashes,
    ) -> [u8; 64] {
        transaction_hasher_v1("Signature")
            .chain(&schema_version)
            .chain(seal_signer)
            .chain(&fields)
            .chain(blob_hashes)
            .result()
    }

    pub fn verify_v1_pruned(
        &self,
        seal_signer: &RistrettoPublicKeyBytes,
        transaction: &PrunedUnsignedTransactionV1,
        blob_hashes: &BlobHashes,
    ) -> bool {
        self.verify_message(Self::create_message_v1_pruned(seal_signer, transaction, blob_hashes))
    }

    /// True when every signature in `signatures` verifies against `message`.
    ///
    /// Accepts exactly the sets [`Self::verify_message`] accepts one by one, but folds the checks
    /// into a single multiscalar multiplication (see [`verify_batch`]). A rejection is a verdict on
    /// the set and does not say which signature is at fault; that costs a pass of its own, and
    /// nothing downstream of a rejected transaction needs it.
    ///
    /// A caller verifying a sealed transaction should use [`verify_sealed_batch`] instead, which
    /// folds the seal into the same multiplication.
    pub fn verify_all_against_message(signatures: &[Self], message: [u8; 64]) -> bool {
        if signatures.is_empty() {
            return true;
        }
        verify_batch(signatures.iter().map(|sig| (&sig.public_key, &sig.signature, &message)))
    }
}

/// True when a sealed transaction's whole signature set — its seal and every authorization —
/// verifies, checked in one batch.
///
/// The two kinds sign different messages, which the batch equation is indifferent to: each term
/// derives its own challenge from its own message, so a shared message saves deriving the digest
/// more than once but is not what makes the fold sound. Folding the seal in replaces a standalone
/// constant-time verification with three more terms in a multiplication that was happening anyway.
///
/// The messages are not circular, though the two commitments cross: the seal message commits to the
/// authorization signatures, and the authorization message commits to the seal signer's public key.
/// Authorizations sign over the seal *key* and the seal signs over the finished *signatures*, so
/// both digests are derivable before anything is verified, which is all the batch needs.
pub fn verify_sealed_batch(
    seal: &TransactionSealSignature,
    seal_message: [u8; 64],
    signatures: &[TransactionSignature],
    authorization_message: [u8; 64],
) -> bool {
    verify_batch(
        std::iter::once((seal.public_key(), seal.signature(), &seal_message)).chain(
            signatures
                .iter()
                .map(|sig| (sig.public_key(), sig.signature(), &authorization_message)),
        ),
    )
}

/// True when every `(public key, signature, message)` term verifies.
///
/// Hands the set to [`SigningContext::verify_batch`] under [`TRANSACTION_SIGNING_CONTEXT`], which folds the n
/// verification equations into one variable-time multiscalar multiplication under weights derived from the whole set.
/// The weights are a transcript of the set rather than local randomness, so every node reaches the same verdict on the
/// same bytes — a validity predicate a committee votes on must not depend on a local RNG. Variable time is safe because
/// every input is public: keys, nonces and scalars travel on the wire and the messages are digests of a gossiped
/// transaction body.
///
/// The messages are per term, so a transaction's seal batches together with the authorizations that
/// sign a different digest. A term that fails to decode, or whose key is the identity, rejects the
/// set. A rejection is a verdict on the set and does not say which signature is at fault. An empty
/// batch is rejected, so [`TransactionSignature::verify_all_against_message`] accepts a transaction
/// with no authorizations before calling this, and [`verify_sealed_batch`] always has the seal.
///
/// Decoding is the floor under this and the individual checks alike: every term costs a key
/// decompression and a nonce decompression, which no batch equation can amortise. `benches/signature_verification.rs`
/// measures both paths.
///
/// [`SigningContext::verify_batch`]: tari_ootle_common_types::signature::SigningContext::verify_batch
fn verify_batch<'a, I>(terms: I) -> bool
where I: IntoIterator<Item = (&'a RistrettoPublicKeyBytes, &'a Sr25519SignatureBytes, &'a [u8; 64])> {
    let items: Vec<_> = terms
        .into_iter()
        .map(|(public_key, signature, message)| (signature, public_key, message))
        .collect();
    TRANSACTION_SIGNING_CONTEXT.verify_batch(&items)
}

impl From<SignatureOutput> for TransactionSignature {
    fn from(output: SignatureOutput) -> Self {
        Self {
            public_key: output.public_key.to_byte_type(),
            signature: output.signature,
        }
    }
}

/// Field-by-field projection of `UnsignedTransactionV1` used in the signing/id hashing domains.
///
/// Notably *does not* include `blobs` — raw blob bytes never enter signing/id digests. Blob
/// commitments are chained separately as a `BlobHashes` after these fields.
#[derive(Debug, Clone, borsh::BorshSerialize)]
pub(crate) struct TransactionSignatureFields<'a> {
    network: u8,
    fee_instructions: &'a [Instruction],
    instructions: &'a [Instruction],
    inputs: &'a IndexSet<InputDeclaration>,
    min_epoch: Option<Epoch>,
    max_epoch: Epoch,
    main_signer: MainSigner,
    dry_run: bool,
    nonce: u64,
}

impl<'a> From<&'a UnsignedTransactionV1> for TransactionSignatureFields<'a> {
    fn from(transaction: &'a UnsignedTransactionV1) -> Self {
        Self {
            network: transaction.network,
            fee_instructions: &transaction.fee_instructions,
            instructions: &transaction.instructions,
            inputs: &transaction.inputs,
            min_epoch: transaction.min_epoch,
            max_epoch: transaction.max_epoch,
            main_signer: transaction.main_signer,
            dry_run: transaction.dry_run,
            nonce: transaction.nonce,
        }
    }
}

/// Pruned-form projection. Field-by-field identical to the full-form `From` so the borsh
/// encoding (and therefore the digest) is byte-identical.
impl<'a> From<&'a PrunedUnsignedTransactionV1> for TransactionSignatureFields<'a> {
    fn from(transaction: &'a PrunedUnsignedTransactionV1) -> Self {
        Self {
            network: transaction.network,
            fee_instructions: &transaction.fee_instructions,
            instructions: &transaction.instructions,
            inputs: &transaction.inputs,
            min_epoch: transaction.min_epoch,
            max_epoch: transaction.max_epoch,
            main_signer: transaction.main_signer,
            dry_run: transaction.dry_run,
            nonce: transaction.nonce,
        }
    }
}

#[cfg(test)]
mod tests {
    use borsh::BorshSerialize;
    use tari_crypto::keys::SecretKey;
    use tari_engine_types::{SubstateVersion, substate::SubstateId};
    use tari_ootle_common_types::signature::{components, from_components};
    use tari_template_lib_types::ComponentAddress;

    use super::*;

    fn sample_seal_signer() -> RistrettoPublicKeyBytes {
        RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::random(&mut rand::rng())).to_byte_type()
    }

    fn sample_unsigned() -> UnsignedTransactionV1 {
        let mut inputs = IndexSet::new();
        inputs.insert(InputDeclaration::write_versioned(
            SubstateId::Component(ComponentAddress::from_array([1; 32])),
            SubstateVersion::new(1),
        ));
        inputs.insert(InputDeclaration::write_versioned(
            SubstateId::Component(ComponentAddress::from_array([2; 32])),
            SubstateVersion::new(2),
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
            blobs: crate::Blobs::empty(),
            nonce: 5,
        }
    }

    fn random_signature(tx: &UnsignedTransactionV1, seal_signer: &RistrettoPublicKeyBytes) -> TransactionSignature {
        let sk = RistrettoSecretKey::random(&mut rand::rng());
        TransactionSignature::sign_v1(&sk, seal_signer, tx)
    }

    fn sig_msg(seal_signer: &RistrettoPublicKeyBytes, tx: &UnsignedTransactionV1) -> [u8; 64] {
        TransactionSignature::create_message_v1(seal_signer, tx)
    }

    fn seal_msg(t: &UnsealedTransactionV1) -> [u8; 64] {
        TransactionSealSignature::create_message_v1(t)
    }

    /// An authorization produced by an off-the-shelf sr25519 signer under the transaction signing
    /// context verifies, so a signer holding its key in an HSM needs no Ootle-specific code.
    #[test]
    fn an_sr25519_signer_authorizes_a_transaction() {
        use schnorrkel::{ExpansionMode, MiniSecretKey, signing_context};

        let seal_signer = sample_seal_signer();
        let unsigned = sample_unsigned();
        let keypair = MiniSecretKey::from_bytes(&[7; 32])
            .unwrap()
            .expand_to_keypair(ExpansionMode::Ed25519);

        let message = TransactionSignature::create_message_v1(&seal_signer, &unsigned);
        let signature = keypair.sign(signing_context(TRANSACTION_SIGNING_CONTEXT.as_bytes()).bytes(&message));
        let signature = TransactionSignature::new(
            RistrettoPublicKeyBytes::from(keypair.public.to_bytes()),
            Sr25519SignatureBytes::from_array(signature.to_bytes()),
        );

        assert!(signature.verify_v1(&seal_signer, &unsigned));
        assert!(TransactionSignature::verify_all_against_message(
            std::slice::from_ref(&signature),
            message
        ));
    }

    #[test]
    fn signature_message_is_deterministic() {
        let signer = sample_seal_signer();
        let tx = sample_unsigned();
        assert_eq!(sig_msg(&signer, &tx), sig_msg(&signer, &tx));
    }

    /// Every field of the signed message (seal signer + every field of UnsignedTransactionV1) must
    /// influence the digest. A failure here means a tx field has escaped the signing domain and
    /// signatures are malleable with respect to it.
    #[test]
    fn signature_message_binds_all_fields() {
        let signer = sample_seal_signer();
        let base = sample_unsigned();
        let base_msg = sig_msg(&signer, &base);

        // seal_signer context
        let other_signer = sample_seal_signer();
        assert_ne!(sig_msg(&other_signer, &base), base_msg, "seal_signer");

        // network
        let mut tx = base.clone();
        tx.network = tx.network.wrapping_add(1);
        assert_ne!(sig_msg(&signer, &tx), base_msg, "network");

        // fee_instructions: extra / empty
        let mut tx = base.clone();
        tx.fee_instructions.push(Instruction::DropAllProofsInWorkspace);
        assert_ne!(sig_msg(&signer, &tx), base_msg, "fee_instructions (extra)");
        let mut tx = base.clone();
        tx.fee_instructions.clear();
        assert_ne!(sig_msg(&signer, &tx), base_msg, "fee_instructions (empty)");

        // instructions: extra / reordered
        let mut tx = base.clone();
        tx.instructions.push(Instruction::DropAllProofsInWorkspace);
        assert_ne!(sig_msg(&signer, &tx), base_msg, "instructions (extra)");
        let mut tx = base.clone();
        tx.instructions.reverse();
        assert_ne!(sig_msg(&signer, &tx), base_msg, "instructions (reordered)");

        // inputs: extra / reorder / version changed
        let mut tx = base.clone();
        tx.inputs.insert(InputDeclaration::write_versioned(
            SubstateId::Component(ComponentAddress::from_array([9; 32])),
            SubstateVersion::new(1),
        ));
        assert_ne!(sig_msg(&signer, &tx), base_msg, "inputs (extra)");

        let mut tx = base.clone();
        tx.inputs = tx.inputs.iter().rev().cloned().collect();
        assert_ne!(sig_msg(&signer, &tx), base_msg, "inputs (reordered)");

        let mut tx = base.clone();
        tx.inputs = base
            .inputs
            .iter()
            .map(|i| InputDeclaration {
                substate_id: i.substate_id.clone(),
                version: i.version.map(|v| v.next()),
                is_write: i.is_write,
            })
            .collect();
        assert_ne!(sig_msg(&signer, &tx), base_msg, "inputs (version changed)");
        let mut tx = base.clone();
        tx.inputs = base
            .inputs
            .iter()
            .map(|i| i.clone().with_is_write(!i.is_write))
            .collect();
        assert_ne!(sig_msg(&signer, &tx), base_msg, "inputs (intent changed)");

        // min_epoch: value change / Some <-> None
        let mut tx = base.clone();
        tx.min_epoch = Some(Epoch(101));
        assert_ne!(sig_msg(&signer, &tx), base_msg, "min_epoch (value)");
        let mut tx = base.clone();
        tx.min_epoch = None;
        assert_ne!(sig_msg(&signer, &tx), base_msg, "min_epoch (None)");

        // max_epoch
        let mut tx = base.clone();
        tx.max_epoch = Epoch(999);
        assert_ne!(sig_msg(&signer, &tx), base_msg, "max_epoch (value)");

        // main_signer
        for main_signer in crate::v1::main_signer::other_main_signers(base.main_signer) {
            let mut tx = base.clone();
            tx.main_signer = main_signer;
            assert_ne!(sig_msg(&signer, &tx), base_msg, "main_signer {main_signer:?}");
        }

        // dry_run
        let mut tx = base.clone();
        tx.dry_run = !tx.dry_run;
        assert_ne!(sig_msg(&signer, &tx), base_msg, "dry_run");

        // nonce
        let mut tx = base.clone();
        tx.nonce = tx.nonce.wrapping_add(1);
        assert_ne!(sig_msg(&signer, &tx), base_msg, "nonce");

        // blobs: payload contents must influence the digest via per-blob commitments
        let mut tx = base.clone();
        tx.blobs.push(crate::Blob::from(vec![1, 2, 3])).unwrap();
        assert_ne!(sig_msg(&signer, &tx), base_msg, "blobs (added)");

        // Two transactions with the same blob count but different bytes must differ.
        let mut a = base.clone();
        a.blobs.push(crate::Blob::from(vec![1])).unwrap();
        let mut b = base.clone();
        b.blobs.push(crate::Blob::from(vec![2])).unwrap();
        assert_ne!(sig_msg(&signer, &a), sig_msg(&signer, &b), "blobs (contents)");
    }

    #[test]
    fn signature_is_bound_to_seal_signer_context() {
        let signer_sk = RistrettoSecretKey::random(&mut rand::rng());
        let seal_signer_pk = sample_seal_signer();
        let other_seal_signer_pk = sample_seal_signer();
        let tx = sample_unsigned();

        let sig = TransactionSignature::sign_v1(&signer_sk, &seal_signer_pk, &tx);
        assert!(sig.verify_v1(&seal_signer_pk, &tx));
        assert!(
            !sig.verify_v1(&other_seal_signer_pk, &tx),
            "a signature made under one seal signer must not verify under another",
        );

        let mut mutated = tx.clone();
        mutated.dry_run = !mutated.dry_run;
        assert!(!sig.verify_v1(&seal_signer_pk, &mutated));
    }

    fn unsealed_with(unsigned: UnsignedTransactionV1, sigs: Vec<TransactionSignature>) -> UnsealedTransactionV1 {
        UnsealedTransactionV1::new(unsigned, sigs)
    }

    #[test]
    fn seal_message_is_deterministic() {
        let seal_signer = sample_seal_signer();
        let unsigned = sample_unsigned();
        let sig = random_signature(&unsigned, &seal_signer);
        let a = unsealed_with(unsigned.clone(), vec![sig.clone()]);
        let b = unsealed_with(unsigned, vec![sig]);
        assert_eq!(seal_msg(&a), seal_msg(&b));
    }

    /// Every field of UnsignedTransactionV1 reached via the seal message must influence the digest.
    #[test]
    fn seal_message_binds_all_unsigned_fields() {
        let seal_signer = sample_seal_signer();
        let base_unsigned = sample_unsigned();
        let sigs = vec![random_signature(&base_unsigned, &seal_signer)];
        let base = unsealed_with(base_unsigned.clone(), sigs.clone());
        let base_msg = seal_msg(&base);

        let with_body = |u: UnsignedTransactionV1| unsealed_with(u, sigs.clone());

        // network
        let mut u = base_unsigned.clone();
        u.network = u.network.wrapping_add(1);
        assert_ne!(seal_msg(&with_body(u)), base_msg, "network");

        // fee_instructions
        let mut u = base_unsigned.clone();
        u.fee_instructions.push(Instruction::DropAllProofsInWorkspace);
        assert_ne!(seal_msg(&with_body(u)), base_msg, "fee_instructions");

        // instructions: extra / reordered
        let mut u = base_unsigned.clone();
        u.instructions.push(Instruction::DropAllProofsInWorkspace);
        assert_ne!(seal_msg(&with_body(u)), base_msg, "instructions (extra)");
        let mut u = base_unsigned.clone();
        u.instructions.reverse();
        assert_ne!(seal_msg(&with_body(u)), base_msg, "instructions (reordered)");

        // inputs: extra / reordered / version changed
        let mut u = base_unsigned.clone();
        u.inputs.insert(InputDeclaration::write_versioned(
            SubstateId::Component(ComponentAddress::from_array([9; 32])),
            SubstateVersion::new(1),
        ));
        assert_ne!(seal_msg(&with_body(u)), base_msg, "inputs (extra)");

        let mut u = base_unsigned.clone();
        u.inputs = u.inputs.iter().rev().cloned().collect();
        assert_ne!(seal_msg(&with_body(u)), base_msg, "inputs (reordered)");

        let mut u = base_unsigned.clone();
        u.inputs = base_unsigned
            .inputs
            .iter()
            .map(|i| InputDeclaration {
                substate_id: i.substate_id.clone(),
                version: i.version.map(|v| v.next()),
                is_write: i.is_write,
            })
            .collect();
        assert_ne!(seal_msg(&with_body(u)), base_msg, "inputs (version changed)");
        let mut u = base_unsigned.clone();
        u.inputs = base_unsigned
            .inputs
            .iter()
            .map(|i| i.clone().with_is_write(!i.is_write))
            .collect();
        assert_ne!(seal_msg(&with_body(u)), base_msg, "inputs (intent changed)");

        // min_epoch
        let mut u = base_unsigned.clone();
        u.min_epoch = Some(Epoch(101));
        assert_ne!(seal_msg(&with_body(u)), base_msg, "min_epoch (value)");
        let mut u = base_unsigned.clone();
        u.min_epoch = None;
        assert_ne!(seal_msg(&with_body(u)), base_msg, "min_epoch (None)");

        // max_epoch
        let mut u = base_unsigned.clone();
        u.max_epoch = Epoch(999);
        assert_ne!(seal_msg(&with_body(u)), base_msg, "max_epoch (value)");

        // main_signer
        for main_signer in crate::v1::main_signer::other_main_signers(base_unsigned.main_signer) {
            let mut u = base_unsigned.clone();
            u.main_signer = main_signer;
            assert_ne!(seal_msg(&with_body(u)), base_msg, "main_signer {main_signer:?}");
        }

        // dry_run
        let mut u = base_unsigned.clone();
        u.dry_run = !u.dry_run;
        assert_ne!(seal_msg(&with_body(u)), base_msg, "dry_run");

        // nonce
        let mut u = base_unsigned.clone();
        u.nonce = u.nonce.wrapping_add(1);
        assert_ne!(seal_msg(&with_body(u)), base_msg, "nonce");

        // blobs: changes to the payload must alter the seal digest via per-blob commitments
        let mut u = base_unsigned.clone();
        u.blobs.push(crate::Blob::from(vec![9, 9, 9])).unwrap();
        assert_ne!(seal_msg(&with_body(u)), base_msg, "blobs (added)");
    }

    /// The seal signature binds the prior signatures — their presence, content and order must all
    /// affect the seal digest, otherwise a seal could be lifted onto a transaction with forged
    /// signatures.
    #[test]
    fn seal_message_binds_signatures() {
        let seal_signer = sample_seal_signer();
        let unsigned = sample_unsigned();

        let sig1 = random_signature(&unsigned, &seal_signer);
        let sig2 = random_signature(&unsigned, &seal_signer);

        let no_sigs = unsealed_with(unsigned.clone(), vec![]);
        let with_sig1 = unsealed_with(unsigned.clone(), vec![sig1.clone()]);
        let with_sig2 = unsealed_with(unsigned.clone(), vec![sig2.clone()]);
        let ab = unsealed_with(unsigned.clone(), vec![sig1.clone(), sig2.clone()]);
        let ba = unsealed_with(unsigned, vec![sig2, sig1]);

        assert_ne!(seal_msg(&no_sigs), seal_msg(&with_sig1), "count (0 vs 1)");
        assert_ne!(seal_msg(&with_sig1), seal_msg(&with_sig2), "content");
        assert_ne!(seal_msg(&with_sig1), seal_msg(&ab), "count (1 vs 2)");
        assert_ne!(seal_msg(&ab), seal_msg(&ba), "order");
    }

    /// The streamed preimage segments must concatenate to exactly the byte sequence that
    /// `TransactionSignature::create_message_v1_inner` chains into the hasher. If this drifts, a
    /// hardware signer would compute a different digest and produce signatures that fail to verify.
    #[test]
    fn signature_preimage_matches_chain_order() {
        let signer = sample_seal_signer();
        let tx = sample_unsigned();

        let reconstructed: Vec<u8> = TransactionSignature::signing_preimage_v1(&signer, &tx)
            .iter()
            .flat_map(|s| s.bytes.clone())
            .collect();

        // Mirror create_message_v1_inner exactly: schema_version, seal_signer, fields, blob_hashes.
        let mut expected = Vec::new();
        BorshSerialize::serialize(&tx.schema_version(), &mut expected).unwrap();
        BorshSerialize::serialize(&signer, &mut expected).unwrap();
        BorshSerialize::serialize(&TransactionSignatureFields::from(&tx), &mut expected).unwrap();
        BorshSerialize::serialize(&tx.blobs.hashes(), &mut expected).unwrap();

        assert_eq!(reconstructed, expected);
    }

    /// As above, for the seal ("SealSignature") preimage: schema_version, fields, blob_hashes,
    /// signatures.
    #[test]
    fn seal_preimage_matches_chain_order() {
        let seal_signer = sample_seal_signer();
        let unsigned = sample_unsigned();
        let sig = random_signature(&unsigned, &seal_signer);
        let unsealed = unsealed_with(unsigned.clone(), vec![sig]);

        let reconstructed: Vec<u8> = TransactionSealSignature::signing_preimage_v1(&unsealed)
            .iter()
            .flat_map(|s| s.bytes.clone())
            .collect();

        let mut expected = Vec::new();
        BorshSerialize::serialize(&unsealed.schema_version(), &mut expected).unwrap();
        BorshSerialize::serialize(&TransactionSignatureFields::from(&unsigned), &mut expected).unwrap();
        BorshSerialize::serialize(&unsigned.blobs.hashes(), &mut expected).unwrap();
        BorshSerialize::serialize(&unsealed.signatures(), &mut expected).unwrap();

        assert_eq!(reconstructed, expected);
    }

    /// The `_with_blob_hashes` entry points let a caller derive blob commitments once and share
    /// them between the seal and authorization messages. They must produce digests identical to
    /// deriving the commitments inline — otherwise a signature would verify on one path and fail on
    /// the other.
    #[test]
    fn with_blob_hashes_matches_inline_derivation() {
        let seal_signer = sample_seal_signer();
        let mut unsigned = sample_unsigned();
        unsigned.blobs.push(crate::Blob::from(vec![4, 5, 6])).unwrap();
        let sig = random_signature(&unsigned, &seal_signer);
        let unsealed = unsealed_with(unsigned.clone(), vec![sig]);
        let blob_hashes = unsigned.blobs.hashes();

        assert_eq!(
            TransactionSignature::create_message_v1(&seal_signer, &unsigned),
            TransactionSignature::create_message_v1_with_blob_hashes(&seal_signer, &unsigned, &blob_hashes),
            "authorization message",
        );
        assert_eq!(
            TransactionSealSignature::create_message_v1(&unsealed),
            TransactionSealSignature::create_message_v1_with_blob_hashes(&unsealed, &blob_hashes),
            "seal message",
        );
    }

    /// The signing message is derived once for the whole signature set, so every signature must
    /// still be checked against it individually: a fully valid set verifies, and a signature over a
    /// different body fails at any position in the set.
    #[test]
    fn verify_all_signatures_checks_every_signature() {
        let seal_signer = sample_seal_signer();
        let unsigned = sample_unsigned();

        let sigs: Vec<_> = (0..4).map(|_| random_signature(&unsigned, &seal_signer)).collect();
        let all_valid = unsealed_with(unsigned.clone(), sigs.clone());
        assert!(all_valid.verify_all_signatures(&seal_signer));

        let mut other_body = unsigned.clone();
        other_body.dry_run = !other_body.dry_run;
        let foreign = random_signature(&other_body, &seal_signer);

        for i in 0..sigs.len() {
            let mut planted = sigs.clone();
            planted[i] = foreign.clone();
            assert!(
                !unsealed_with(unsigned.clone(), planted).verify_all_signatures(&seal_signer),
                "a signature over a different body at index {i} must fail verification",
            );
        }

        assert!(
            !all_valid.verify_all_signatures(&sample_seal_signer()),
            "the set is bound to the seal signer it was signed under",
        );
    }

    /// Exchanges the `s` scalars of two valid signatures, leaving each key and nonce in place. The
    /// challenge binds only the nonce and the key, so each result is invalid by exactly the
    /// difference of the two scalars — equal and opposite error terms, which is what a cancelling
    /// pair needs.
    fn swap_scalars(a: &TransactionSignature, b: &TransactionSignature) -> Vec<TransactionSignature> {
        let (a_nonce, a_scalar) = components(a.signature()).unwrap();
        let (b_nonce, b_scalar) = components(b.signature()).unwrap();
        vec![
            TransactionSignature::new(*a.public_key(), from_components(&a_nonce, &b_scalar)),
            TransactionSignature::new(*b.public_key(), from_components(&b_nonce, &a_scalar)),
        ]
    }

    /// The batch must accept exactly the sets the individual checks accept, and name the same
    /// signature when it rejects — at a single signature, at the sizes where the multiscalar
    /// multiplication switches algorithm, and above.
    #[test]
    fn batch_verification_agrees_with_individual_checks() {
        let seal_signer = sample_seal_signer();
        let unsigned = sample_unsigned();
        let message = sig_msg(&seal_signer, &unsigned);

        let mut other_body = unsigned.clone();
        other_body.dry_run = !other_body.dry_run;
        let foreign = random_signature(&other_body, &seal_signer);

        for n in [1usize, 2, 3, 4, 8, 33] {
            let sigs: Vec<_> = (0..n).map(|_| random_signature(&unsigned, &seal_signer)).collect();
            assert!(
                TransactionSignature::verify_all_against_message(&sigs, message),
                "{n} valid signatures must verify",
            );

            // Every position for the small sets; the ends and the middle for the large one, whose
            // per-index cost is quadratic and adds no coverage the small sets do not already give.
            let planting_positions: Vec<usize> = if n <= 8 {
                (0..n).collect()
            } else {
                vec![0, n / 2, n - 1]
            };
            for i in planting_positions {
                let mut planted = sigs.clone();
                planted[i] = foreign.clone();
                assert!(
                    !planted[i].verify_message(message),
                    "the planted signature at index {i} must not verify on its own",
                );
                assert!(
                    !TransactionSignature::verify_all_against_message(&planted, message),
                    "a foreign signature at index {i} of {n} must be rejected",
                );
            }
        }
    }

    /// The reason the weights exist. Two invalid signatures whose error terms are `+delta·G` and
    /// `-delta·G` satisfy the summed equation exactly when both errors carry the same coefficient,
    /// so an unweighted batch accepts a pair that neither individual check would. The weights make
    /// the errors cancel only for a `zᵢ` vector the pair would have to be searched for.
    #[test]
    fn batch_verification_rejects_cancelling_errors() {
        let seal_signer = sample_seal_signer();
        let unsigned = sample_unsigned();
        let message = sig_msg(&seal_signer, &unsigned);

        let first = random_signature(&unsigned, &seal_signer);
        let second = random_signature(&unsigned, &seal_signer);
        let pair = swap_scalars(&first, &second);

        // Neither is a valid signature on its own.
        for (i, sig) in pair.iter().enumerate() {
            assert!(!sig.verify_message(message), "offset signature {i} must not verify");
        }

        // The unweighted sum of their verification equations nonetheless balances: sum(sᵢ)·G ==
        // sum(Rᵢ) + sum(eᵢ·Pᵢ). Asserting it is what pins the test to the property under test — if
        // this ever stops holding, the case below no longer exercises the weights at all.
        let mut lhs = RistrettoSecretKey::default();
        let mut rhs = RistrettoPublicKey::default();
        for sig in &pair {
            let public_key = RistrettoPublicKey::from_canonical_bytes(sig.public_key().as_bytes()).unwrap();
            let (nonce, scalar) = components(sig.signature()).unwrap();
            let e = TRANSACTION_SIGNING_CONTEXT.challenge(&nonce, &public_key, &message);
            lhs = &lhs + &scalar;
            rhs = &(&rhs + &nonce) + &(&e * &public_key);
        }
        assert_eq!(
            RistrettoPublicKey::from_secret_key(&lhs),
            rhs,
            "the pair must balance unweighted, or it does not test the weights",
        );

        assert!(
            !TransactionSignature::verify_all_against_message(&pair, message),
            "a pair whose errors cancel unweighted must still be rejected",
        );
    }

    /// The batch must bind each key to its own nonce and scalar. Verifying the same components under
    /// a permutation would mean a signer could authorize with another's key.
    #[test]
    fn batch_verification_rejects_permuted_components() {
        let seal_signer = sample_seal_signer();
        let unsigned = sample_unsigned();
        let message = sig_msg(&seal_signer, &unsigned);

        let first = random_signature(&unsigned, &seal_signer);
        let second = random_signature(&unsigned, &seal_signer);
        let swapped = vec![
            TransactionSignature::new(*second.public_key(), *first.signature()),
            TransactionSignature::new(*first.public_key(), *second.signature()),
        ];

        assert!(
            !TransactionSignature::verify_all_against_message(&swapped, message),
            "signatures verified under each other's keys must be rejected",
        );
    }

    /// Bytes that decode to neither a key nor a signature are rejected rather than panicking, at any
    /// position in the set.
    #[test]
    fn batch_verification_rejects_undecodable_components() {
        let seal_signer = sample_seal_signer();
        let unsigned = sample_unsigned();
        let message = sig_msg(&seal_signer, &unsigned);

        let valid = random_signature(&unsigned, &seal_signer);
        let bad_key = TransactionSignature::new(
            RistrettoPublicKeyBytes::from_bytes(&[0xff; 32]).unwrap(),
            *valid.signature(),
        );
        let bad_signature =
            TransactionSignature::new(*valid.public_key(), Sr25519SignatureBytes::from_array([0xff; 64]));

        for (label, planted) in [
            ("key", vec![bad_key, valid.clone()]),
            ("signature", vec![valid.clone(), bad_signature]),
        ] {
            assert!(
                !TransactionSignature::verify_all_against_message(&planted, message),
                "an undecodable {label} must be rejected",
            );
        }
    }

    /// The identity public key must be rejected.
    ///
    /// Its encoding is 32 zero bytes, which decompresses perfectly well, and with `P = 0` the
    /// verification equation degenerates to `s·G == R` — satisfied by anyone who picks `s` and sets
    /// `R = s·G`. So the batch must not accept a term the individual check would refuse: the batch
    /// short-circuits on success, and an accepted set never reaches the individual checks at all.
    #[test]
    fn batch_verification_rejects_the_identity_public_key() {
        let seal_signer = sample_seal_signer();
        let unsigned = sample_unsigned();
        let message = sig_msg(&seal_signer, &unsigned);

        // A signature that satisfies `s·G == R` under the identity key: sign anything with a key of
        // our choosing and keep the `(R, s)` pair, whose relation holds for whatever key we name.
        let nonce = RistrettoSecretKey::random(&mut rand::rng());
        let forged = TransactionSignature::new(
            RistrettoPublicKeyBytes::zero(),
            from_components(&RistrettoPublicKey::from_secret_key(&nonce), &nonce),
        );

        assert!(
            !forged.verify_message(message),
            "the individual check must refuse the identity key, or this test proves nothing",
        );
        assert!(
            !TransactionSignature::verify_all_against_message(std::slice::from_ref(&forged), message),
            "the batch must refuse what the individual check refuses",
        );

        let valid = random_signature(&unsigned, &seal_signer);
        assert!(
            !TransactionSignature::verify_all_against_message(&[valid, forged], message),
            "an identity key alongside a valid signature must still be refused",
        );
    }

    /// The seal verifies in the same batch as the authorizations despite signing a different
    /// message, and an invalid signature of either kind is refused.
    #[test]
    fn sealed_batch_verifies_both_kinds() {
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let seal_signer: RistrettoPublicKeyBytes = RistrettoPublicKey::from_secret_key(&sealer).to_byte_type();
        let unsigned = sample_unsigned();

        for n in [0usize, 1, 4] {
            let sigs: Vec<_> = (0..n).map(|_| random_signature(&unsigned, &seal_signer)).collect();
            let unsealed = unsealed_with(unsigned.clone(), sigs.clone());
            let seal = TransactionSealSignature::sign_v1(&sealer, &unsealed);
            let seal_message = seal_msg(&unsealed);
            let authorization_message = sig_msg(&seal_signer, &unsigned);

            assert!(
                verify_sealed_batch(&seal, seal_message, &sigs, authorization_message),
                "a seal over {n} authorizations must verify in one batch",
            );

            // A seal by the same key over a different body. Sealing with another key instead would
            // not be a forgery — the seal carries its own public key, so it would simply be a
            // transaction sealed by someone else.
            let mut other_body = unsigned.clone();
            other_body.dry_run = !other_body.dry_run;
            let stale_seal =
                TransactionSealSignature::sign_v1(&sealer, &unsealed_with(other_body.clone(), sigs.clone()));
            assert!(
                !verify_sealed_batch(&stale_seal, seal_message, &sigs, authorization_message),
                "a seal over a different body must be refused, with {n} authorizations present",
            );

            let foreign = random_signature(&other_body, &seal_signer);
            for i in 0..n {
                let mut planted = sigs.clone();
                planted[i] = foreign.clone();
                assert!(
                    !verify_sealed_batch(&seal, seal_message, &planted, authorization_message),
                    "a foreign authorization at index {i} of {n} must be refused",
                );
            }
        }
    }

    /// The seal and an authorization must not be able to cover for each other: a cancelling pair
    /// straddling the two kinds is rejected exactly as one within the authorizations is.
    #[test]
    fn sealed_batch_rejects_errors_cancelling_across_the_seal() {
        let sealer = RistrettoSecretKey::random(&mut rand::rng());
        let seal_signer: RistrettoPublicKeyBytes = RistrettoPublicKey::from_secret_key(&sealer).to_byte_type();
        let unsigned = sample_unsigned();

        let authorization = random_signature(&unsigned, &seal_signer);
        let unsealed = unsealed_with(unsigned.clone(), vec![authorization.clone()]);
        let seal = TransactionSealSignature::sign_v1(&sealer, &unsealed);

        // Exchange the scalars of the seal and the authorization, leaving each key and nonce in
        // place: equal and opposite error terms, one on each side of the seal boundary.
        let swapped = swap_scalars(
            &TransactionSignature::new(*seal.public_key(), *seal.signature()),
            &authorization,
        );
        let crossed_seal = TransactionSealSignature::new(*swapped[0].public_key(), *swapped[0].signature());

        assert!(
            !verify_sealed_batch(
                &crossed_seal,
                seal_msg(&unsealed),
                &swapped[1..],
                sig_msg(&seal_signer, &unsigned),
            ),
            "errors that cancel across the seal must still be rejected",
        );
    }

    #[test]
    fn seal_signature_roundtrip() {
        let sealer_sk = RistrettoSecretKey::random(&mut rand::rng());
        let seal_signer_pk: RistrettoPublicKeyBytes = RistrettoPublicKey::from_secret_key(&sealer_sk).to_byte_type();

        let unsigned = sample_unsigned();
        let sig = random_signature(&unsigned, &seal_signer_pk);
        let t = unsealed_with(unsigned, vec![sig]);

        let seal = TransactionSealSignature::sign_v1(&sealer_sk, &t);
        assert!(seal.verify_v1(&t));

        // Mutating a body field breaks the seal.
        let mut mutated_inner = t.unsigned_transaction().clone();
        mutated_inner.dry_run = !mutated_inner.dry_run;
        let mutated = UnsealedTransactionV1::new(mutated_inner, t.signatures().to_vec());
        assert!(!seal.verify_v1(&mutated));

        // Mutating signatures also breaks the seal.
        let extra_sig = random_signature(t.unsigned_transaction(), &seal_signer_pk);
        let mut sigs = t.signatures().to_vec();
        sigs.push(extra_sig);
        let mutated = UnsealedTransactionV1::new(t.unsigned_transaction().clone(), sigs);
        assert!(!seal.verify_v1(&mutated));
    }
}
