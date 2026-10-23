//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Host reference for the on-device signing recipe (`ootle-ledger-app/src/hashing.rs`).
//!
//! This independently reimplements, using the `blake2` crate + `curve25519-dalek`, the exact byte
//! recipe the Ledger device follows to (1) recompute a transaction's 64-byte signing message from
//! the streamed preimage segments and (2) produce an sr25519 signature. It then proves
//! the result against `tari_ootle_transaction` / `tari_crypto`:
//!   - the reference message equals `create_message_v1` (domain separation + borsh feeding), and
//!   - the reference signature verifies via `verify_v1` (sr25519 challenge + `s = e*k + r`).
//!
//! If this passes, the device recipe is correct by construction; only the SDK's Blake2b and the
//! APDU transport remain to be exercised on Speculos.

use blake2::{Blake2b512, Digest};
use curve25519_dalek::{Scalar, constants::RISTRETTO_BASEPOINT_TABLE, ristretto::CompressedRistretto};
use indexmap::IndexSet;
use ootle_ledger_common::signing::{
    NONCE_DOMAIN,
    SIGNING_CONTEXT,
    STEALTH_OWNER_LABEL,
    TX_DOMAIN,
    TX_DOMAIN_VERSION,
    TX_LABEL_SEAL,
    TX_LABEL_SIGNATURE,
    WALLET_DOMAIN,
    WALLET_DOMAIN_VERSION,
};
use rand::Rng;
use tari_ootle_common_types::signature::Sr25519SignatureBytes;
use tari_ootle_transaction::{
    Blob,
    Blobs,
    Epoch,
    Instruction,
    MainSigner,
    PreimageSegment,
    TransactionSealSignature,
    TransactionSignature,
    UnsealedTransactionV1,
    UnsignedTransactionV1,
};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

// ---- reference reimplementation of the device recipe (mirrors src/hashing.rs) -------------------

fn byte_to_decimal_ascii_bytes(mut byte: u8) -> (usize, [u8; 3]) {
    const ZERO: u8 = 48;
    let mut bytes = [0u8, 0u8, ZERO];
    let mut pos = 3usize;
    if byte == 0 {
        return (2, bytes);
    }
    while byte > 0 {
        bytes[pos - 1] = ZERO + (byte % 10);
        byte /= 10;
        pos -= 1;
    }
    (pos, bytes)
}

fn add_domain_separation_tag(h: &mut Blake2b512, domain: &str, version: u8, label: &str) {
    let (off, ascii) = byte_to_decimal_ascii_bytes(version);
    let version_len = 3 - off;
    let len = if label.is_empty() {
        domain.len() + version_len + 2
    } else {
        domain.len() + version_len + label.len() + 3
    };
    h.update((len as u64).to_le_bytes());
    h.update(domain.as_bytes());
    h.update(b".v");
    h.update(&ascii[off..]);
    if !label.is_empty() {
        h.update(b".");
        h.update(label.as_bytes());
    }
}

/// Recompute the transaction message: domain preamble, then each preimage segment fed straight
/// into the hasher (mirrors the device, which streams chunks without buffering the whole preimage).
fn message(label: &str, segments: &[PreimageSegment]) -> [u8; 64] {
    let mut h = Blake2b512::new();
    add_domain_separation_tag(&mut h, TX_DOMAIN, TX_DOMAIN_VERSION, label);
    for segment in segments {
        h.update(&segment.bytes);
    }
    let mut out = [0u8; 64];
    out.copy_from_slice(&h.finalize());
    out
}

fn schnorr_challenge(r: &[u8; 32], p: &[u8; 32], m: &[u8; 64]) -> Scalar {
    let mut transcript = merlin::Transcript::new(b"SigningContext");
    transcript.append_message(b"", SIGNING_CONTEXT);
    transcript.append_message(b"sign-bytes", m);
    transcript.append_message(b"proto-name", b"Schnorr-sig");
    transcript.append_message(b"sign:pk", p);
    transcript.append_message(b"sign:R", r);
    let mut out = [0u8; 64];
    transcript.challenge_bytes(b"sign:c", &mut out);
    Scalar::from_bytes_mod_order_wide(&out)
}

fn deterministic_nonce(secret: &Scalar, m: &[u8; 64]) -> Scalar {
    let mut h = Blake2b512::new();
    h.update(NONCE_DOMAIN);
    h.update(secret.as_bytes());
    h.update(m);
    let mut out = [0u8; 64];
    out.copy_from_slice(&h.finalize());
    Scalar::from_bytes_mod_order_wide(&out)
}

fn public_key(s: &Scalar) -> [u8; 32] {
    (s * RISTRETTO_BASEPOINT_TABLE).compress().0
}

fn sign(secret: &Scalar, m: &[u8; 64]) -> ([u8; 32], [u8; 64]) {
    let pk = public_key(secret);
    let nonce = deterministic_nonce(secret, m);
    let r = public_key(&nonce);
    let e = schnorr_challenge(&r, &pk, m);
    let s = e * secret + nonce;
    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&r);
    sig[32..].copy_from_slice(s.as_bytes());
    sig[63] |= 0x80;
    (pk, sig)
}

/// The stealth signing-key tweak applied before signing a confidential transfer (mirrors
/// `derive_stealth_secret` in `src/hashing.rs`): `c + k` where `c = H_stealth_owner(network, v·R)`,
/// `v` is the account's view secret and `R` is the spent UTXO's sender public nonce. The DH point is
/// compressed and fed borsh-encoded (`u32_le(len) || bytes`), matching
/// `DomainSeparatedBorshHasher::update_consensus_encode` on a byte slice in
/// `tari_ootle_wallet_crypto::kdfs::dh_kdf_aead`.
fn derive_stealth_secret(
    network_byte: u8,
    account_secret: &Scalar,
    view_secret: &Scalar,
    public_nonce: &[u8; 32],
) -> Scalar {
    let r = CompressedRistretto(*public_nonce)
        .decompress()
        .expect("public nonce is a valid Ristretto point");
    let dh = (view_secret * r).compress().0;

    let label = format!("{STEALTH_OWNER_LABEL}.n{network_byte}");
    let mut h = Blake2b512::new();
    add_domain_separation_tag(&mut h, WALLET_DOMAIN, WALLET_DOMAIN_VERSION, &label);
    h.update((dh.len() as u32).to_le_bytes());
    h.update(dh);
    let mut out = [0u8; 64];
    out.copy_from_slice(&h.finalize());
    let c = Scalar::from_bytes_mod_order_wide(&out);
    c + account_secret
}

// ---- helpers ------------------------------------------------------------------------------------

fn random_scalar() -> Scalar {
    let mut b = [0u8; 64];
    rand::rng().fill_bytes(&mut b);
    Scalar::from_bytes_mod_order_wide(&b)
}

fn pk_bytes(s: &Scalar) -> RistrettoPublicKeyBytes {
    RistrettoPublicKeyBytes::from_bytes(&public_key(s)).unwrap()
}

fn sig_bytes(sig: &[u8; 64]) -> Sr25519SignatureBytes {
    Sr25519SignatureBytes::from_array(*sig)
}

fn sample_unsigned() -> UnsignedTransactionV1 {
    let mut blobs = Blobs::empty();
    blobs.push(Blob::from(vec![1u8, 2, 3])).unwrap();
    UnsignedTransactionV1 {
        network: 1,
        fee_instructions: vec![Instruction::DropAllProofsInWorkspace],
        instructions: vec![
            Instruction::DropAllProofsInWorkspace,
            Instruction::PutLastInstructionOutputOnWorkspace { key: 7 },
        ],
        inputs: IndexSet::new(),
        min_epoch: None,
        max_epoch: Epoch(1),
        main_signer: MainSigner::None,
        dry_run: true,
        blobs,
        nonce: 42,
    }
}

// ---- tests --------------------------------------------------------------------------------------

#[test]
fn signing_context_matches_protocol() {
    assert_eq!(
        SIGNING_CONTEXT,
        tari_ootle_common_types::signature::TRANSACTION_SIGNING_CONTEXT.as_bytes()
    );
}

#[test]
fn preimage_field_tags_match_protocol() {
    use ootle_ledger_common::arg_types::SigningField;
    use tari_ootle_transaction::PreimageField;

    // The host maps PreimageField -> SigningField by numeric tag, and the device validates field
    // order by the same tags. They must agree.
    let pairs = [
        (PreimageField::SchemaVersion, SigningField::SchemaVersion),
        (PreimageField::SealSigner, SigningField::SealSigner),
        (PreimageField::Network, SigningField::Network),
        (PreimageField::FeeInstructions, SigningField::FeeInstructions),
        (PreimageField::Instructions, SigningField::Instructions),
        (PreimageField::Inputs, SigningField::Inputs),
        (PreimageField::MinEpoch, SigningField::MinEpoch),
        (PreimageField::MaxEpoch, SigningField::MaxEpoch),
        (PreimageField::MainSigner, SigningField::MainSigner),
        (PreimageField::DryRun, SigningField::DryRun),
        (PreimageField::Nonce, SigningField::TransactionNonce),
        (PreimageField::BlobHashes, SigningField::BlobHashes),
        (PreimageField::Signatures, SigningField::Signatures),
    ];
    for (preimage, wire) in pairs {
        assert_eq!(preimage as u8, wire as u8, "tag mismatch for {preimage:?}");
    }
}

#[test]
fn add_signer_recipe_matches_and_verifies() {
    let seal_signer = pk_bytes(&random_scalar());
    let signer = random_scalar();
    let tx = sample_unsigned();

    // (1) message recipe == create_message_v1
    let m = message(
        TX_LABEL_SIGNATURE,
        &TransactionSignature::signing_preimage_v1(&seal_signer, &tx),
    );
    assert_eq!(
        m,
        TransactionSignature::create_message_v1(&seal_signer, &tx),
        "message mismatch"
    );

    // (2) signature produced by the recipe verifies under tari_crypto
    let (pk, sig) = sign(&signer, &m);
    let signature = TransactionSignature::new(RistrettoPublicKeyBytes::from_bytes(&pk).unwrap(), sig_bytes(&sig));
    assert!(
        signature.verify_v1(&seal_signer, &tx),
        "authorization signature failed to verify"
    );
}

#[test]
fn seal_recipe_matches_and_verifies() {
    let unsigned = sample_unsigned();
    let seal_signer = random_scalar();
    let seal_signer_pk = pk_bytes(&seal_signer);

    // A prior authorization signature (its bytes are bound by the seal).
    let signer = random_scalar();
    let auth_msg = message(
        TX_LABEL_SIGNATURE,
        &TransactionSignature::signing_preimage_v1(&seal_signer_pk, &unsigned),
    );
    let (auth_pk, auth_sig) = sign(&signer, &auth_msg);
    let prior = TransactionSignature::new(
        RistrettoPublicKeyBytes::from_bytes(&auth_pk).unwrap(),
        sig_bytes(&auth_sig),
    );

    let unsealed = UnsealedTransactionV1::new(unsigned, vec![prior]);

    // (1) seal message recipe == create_message_v1
    let m = message(TX_LABEL_SEAL, &TransactionSealSignature::signing_preimage_v1(&unsealed));
    assert_eq!(
        m,
        TransactionSealSignature::create_message_v1(&unsealed),
        "seal message mismatch"
    );

    // (2) seal signature verifies
    let (pk, sig) = sign(&seal_signer, &m);
    let seal = TransactionSealSignature::new(RistrettoPublicKeyBytes::from_bytes(&pk).unwrap(), sig_bytes(&sig));
    assert!(seal.verify_v1(&unsealed), "seal signature failed to verify");
}

#[test]
fn stealth_recipe_matches_and_verifies() {
    use ootle_ledger_common::arg_types::{GetPublicKeyRequest, KeyType, StealthTweak};
    use ootle_network::Network;
    use tari_crypto::{
        keys::{PublicKey, SecretKey},
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
        tari_utilities::ByteArray,
    };
    use tari_ootle_wallet_crypto::kdfs::{owner_stealth_dh_secret, owner_stealth_dh_stealth_address};

    let network = Network::LocalNet;
    let mut rng = rand::rng();
    // Account spend key k, view key v, and the sender ephemeral nonce (r, R) carried by the spent UTXO.
    let k = RistrettoSecretKey::random(&mut rng);
    let k_pub = RistrettoPublicKey::from_secret_key(&k);
    let v = RistrettoSecretKey::random(&mut rng);
    let v_pub = RistrettoPublicKey::from_secret_key(&v);
    let r = RistrettoSecretKey::random(&mut rng);
    let r_pub = RistrettoPublicKey::from_secret_key(&r);

    // Device inputs: the account and view scalars and the compressed public nonce R.
    let k_scalar = Scalar::from_bytes_mod_order(<[u8; 32]>::try_from(k.as_bytes()).unwrap());
    let v_scalar = Scalar::from_bytes_mod_order(<[u8; 32]>::try_from(v.as_bytes()).unwrap());
    let r_bytes = <[u8; 32]>::try_from(r_pub.as_bytes()).unwrap();

    // (1) device tweak == owner_stealth_dh_secret
    let got = derive_stealth_secret(network.as_byte(), &k_scalar, &v_scalar, &r_bytes);
    let expected = owner_stealth_dh_secret(network, &k, &v, &r_pub);
    assert_eq!(&got.as_bytes()[..], expected.as_bytes(), "stealth secret mismatch");

    // (2) returned public key == the sender-derived stealth address C = H(r·V)·G + k·G
    let stealth_address = owner_stealth_dh_stealth_address(network, &k_pub, &v_pub, &r);
    assert_eq!(
        public_key(&got),
        <[u8; 32]>::try_from(stealth_address.as_bytes()).unwrap(),
        "stealth address mismatch"
    );

    // (3) an authorization signature produced with the stealth key verifies under tari_crypto
    let seal_signer = pk_bytes(&random_scalar());
    let tx = sample_unsigned();
    let m = message(
        TX_LABEL_SIGNATURE,
        &TransactionSignature::signing_preimage_v1(&seal_signer, &tx),
    );
    let (pk, sig) = sign(&got, &m);
    let signature = TransactionSignature::new(RistrettoPublicKeyBytes::from_bytes(&pk).unwrap(), sig_bytes(&sig));
    assert!(
        signature.verify_v1(&seal_signer, &tx),
        "stealth authorization signature failed to verify"
    );

    // (4) The tweak a `GetPublicKey` request carries must survive the wire and, fed to the same
    // derivation the signing path uses, reproduce the stealth address — that equality is what lets
    // a caller resolve the seal key before any signature binds to it. (Whether the device handler
    // applies the tweak at all is only observable on Speculos.)
    let request = GetPublicKeyRequest {
        account: 0,
        index: 0,
        key_type: KeyType::Account,
        stealth: Some(StealthTweak {
            network: network.as_byte(),
            public_nonce: r_bytes,
        }),
    };
    let decoded: GetPublicKeyRequest = borsh::from_slice(&borsh::to_vec(&request).unwrap()).unwrap();
    let tweak = decoded.stealth.expect("stealth tweak survives the wire");
    let returned = public_key(&derive_stealth_secret(
        tweak.network,
        &k_scalar,
        &v_scalar,
        &tweak.public_nonce,
    ));
    assert_eq!(
        returned,
        <[u8; 32]>::try_from(stealth_address.as_bytes()).unwrap(),
        "GetPublicKey stealth key mismatch"
    );

    // (5) The tweak binds both of its parts: the same nonce on another network, and another nonce
    // on the same network, must each yield a different key.
    let other_network = Network::Igor;
    assert_ne!(
        returned,
        public_key(&derive_stealth_secret(
            other_network.as_byte(),
            &k_scalar,
            &v_scalar,
            &r_bytes
        )),
        "stealth key is not bound to the network"
    );
    let other_nonce =
        <[u8; 32]>::try_from(RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::random(&mut rng)).as_bytes())
            .unwrap();
    assert_ne!(
        returned,
        public_key(&derive_stealth_secret(
            network.as_byte(),
            &k_scalar,
            &v_scalar,
            &other_nonce
        )),
        "stealth key is not bound to the public nonce"
    );
}
