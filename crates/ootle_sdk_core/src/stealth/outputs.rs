//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Stealth **output** construction — builds the per-output witnesses, the aggregated outputs
//! statement, and the aggregated bulletproof. Pure and synchronous.
//!
//! Two entry points per build op:
//!
//! - `*_with_seed(.., &BuildSeed)` — the seed-reproducible path. The seed is expanded by the KDF into a
//!   [`StealthEntropy`] bundle; this path calls **no** RNG of its own. It is reproducible for everything except the
//!   aggregated bulletproof (see below).
//! - the un-suffixed default — expands a fresh OS-RNG seed via [`StealthEntropy::from_os_rng`], then delegates to the
//!   shared body. The OS RNG is the only randomness it adds.
//!
//! ## Why the aggregated bulletproof is not byte-reproducible
//!
//! Byte-for-byte reproducibility of the **aggregated bulletproof** is **not achievable** through the
//! public `tari_crypto` API. `ExtendedRangeProofService::construct_extended_proof(.., Some(seed))`
//! makes the *mask-recovery* nonces (`alpha`, `dL`, `dR`, `d`, `eta`) deterministic, but the final
//! blinding scalars `r`/`s` (which land in the proof as `r1`/`s1`) are **always** drawn from the
//! range-proof transcript RNG, which finalizes the prover's internal `SysRng`
//! (`tari_bulletproofs_plus::range_proof::RangeProof::prove` → `prove_with_rng(.., &mut SysRng)`;
//! `transcripts::build_rng(..).finalize(external_rng)`). Two identical `Some(seed)` calls produce
//! **different** proof bytes. Seeding the proof therefore cannot deliver byte-equal vectors, so the
//! bulletproof is left on its stock (random) path and the stealth golden vectors use **semantic**
//! comparison: the statement validates cryptographically via `validate_stealth_outputs_statement`, and
//! the deterministic fields are compared structurally.
//!
//! Everything that **can** be made deterministic is: the commitment mask and sender nonce are
//! injected directly, and the AEAD nonce + the four ElGamal/ZK nonces are threaded through the
//! additive seeded seams added to `tari_ootle_wallet_crypto`
//! ([`encrypt_data_with_nonce`](tari_ootle_wallet_crypto::encrypted_data::encrypt_data_with_nonce),
//! [`generate_elgamal_viewable_balance_proof_seeded`](tari_ootle_wallet_crypto::viewable_balance_proof::generate_elgamal_viewable_balance_proof_seeded)).
//! So `commitment`, `sender_public_nonce`, `encrypted_data`, `auth`, `tag`, and
//! `viewable_balance_proof` are all byte-reproducible on the deterministic path; only
//! `agg_range_proof` varies run-to-run.

use std::str::FromStr;

use ootle_byte_type::ToByteType;
use ootle_network::Network as InternalNetwork;
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_ootle_wallet_crypto::{
    OutputWitness,
    StealthCryptoApi,
    StealthOutputWitness,
    bullet_proof::generate_extended_bullet_proof,
    encrypted_data::encrypt_data_with_nonce,
    kdfs,
    memo::Memo,
    stealth::condition_root,
    viewable_balance_proof::generate_elgamal_viewable_balance_proof_seeded,
};
use tari_template_lib_types::{
    Amount,
    EncryptedData,
    ResourceAddress,
    access_rules::AccessRule,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
    stealth::{
        RevealedOutput,
        SpendAuthorization,
        SpendCondition,
        StealthOutputsStatement,
        StealthUnspentOutput,
        UnspentOutput,
    },
};

use crate::types::{
    bytes::{BuildSeed, SecretKeyBytes},
    error::OotleSdkError,
    network::Network,
    stealth::{PerOutputEntropy, StealthEntropy, StealthMemo, StealthOutputSpec, StealthPayTo, StealthTransferIntent},
};

/// Parses a boundary public key into the internal [`RistrettoPublicKey`].
fn parse_public_key(bytes: &[u8]) -> Result<RistrettoPublicKey, OotleSdkError> {
    RistrettoPublicKey::from_canonical_bytes(bytes).map_err(|e| OotleSdkError::Key(format!("invalid public key: {e}")))
}

/// Parses a boundary secret scalar (mask/nonce) into the internal [`RistrettoSecretKey`].
fn parse_secret(bytes: &[u8]) -> Result<RistrettoSecretKey, OotleSdkError> {
    RistrettoSecretKey::from_canonical_bytes(bytes)
        .map_err(|e| OotleSdkError::Key(format!("invalid secret scalar: {e}")))
}

/// Maps a boundary [`StealthMemo`] to the internal `Memo`.
fn to_internal_memo(memo: &StealthMemo) -> Result<Memo, OotleSdkError> {
    match memo {
        StealthMemo::Message(s) => {
            Memo::new_message(s.clone()).ok_or_else(|| OotleSdkError::Validation("memo message too long".to_string()))
        },
        StealthMemo::Bytes(b) => {
            Memo::new_bytes(b.clone()).ok_or_else(|| OotleSdkError::Validation("memo bytes too long".to_string()))
        },
    }
}

/// Builds a single stealth output witness from its spec and one pinned [`PerOutputEntropy`] slice.
///
/// The mask, sender nonce, and AEAD nonce are **injected** from the entropy slice rather than drawn
/// from an RNG, so this calls **no** RNG.
///
/// `per_entropy.elgamal_nonce` / `zk_nonces` are not consumed here — the ElGamal viewable-balance
/// proof is built by the statement builder (which threads them) so that the `resource_view_key`
/// presence check and entropy availability are validated in one place.
pub fn build_stealth_output_witness(
    network: Network,
    output_spec: &StealthOutputSpec,
    per_entropy: &PerOutputEntropy,
) -> Result<StealthOutputWitness, OotleSdkError> {
    let internal_network: InternalNetwork = network.into();

    // Fail fast with a clear message rather than a cryptic range-proof error later.
    if output_spec.minimum_value_promise > output_spec.amount {
        return Err(OotleSdkError::Validation(format!(
            "minimum_value_promise ({}) must be <= amount ({})",
            output_spec.minimum_value_promise, output_spec.amount
        )));
    }

    let account_key = parse_public_key(output_spec.destination_account_pk.as_bytes())?;
    let view_key = parse_public_key(output_spec.destination_view_pk.as_bytes())?;
    let resource_address = ResourceAddress::from_str(output_spec.resource_address.as_str())
        .map_err(|e| OotleSdkError::Parse(format!("invalid resource address: {e}")))?;
    let resource_view_key = output_spec
        .resource_view_key
        .as_ref()
        .map(|k| parse_public_key(k.as_bytes()))
        .transpose()?;

    let mask = parse_secret(per_entropy.mask.as_bytes())?;
    let nonce_secret = parse_secret(per_entropy.sender_nonce.as_bytes())?;
    let public_nonce = RistrettoPublicKey::from_secret_key(&nonce_secret);

    let memo = output_spec.memo.as_ref().map(to_internal_memo).transpose()?;

    // The AEAD nonce is the leading 24 bytes of the 32-byte entropy field (the remaining 8 are
    // ignored — see `PerOutputEntropy::aead_nonce`).
    let aead_full = per_entropy.aead_nonce.as_bytes();
    let mut aead_nonce = [0u8; EncryptedData::SIZE_NONCE];
    aead_nonce.copy_from_slice(&aead_full[..EncryptedData::SIZE_NONCE]);

    // Derive the AEAD key exactly as `StealthCryptoApi::encrypt_value_and_mask`, but route through the
    // seeded `encrypt_data_with_nonce` so the injected nonce is honored.
    let encryption_key = kdfs::encrypted_data_dh_kdf_aead(&nonce_secret, &view_key);
    let encrypted_data = encrypt_data_with_nonce(
        output_spec.amount,
        &mask,
        &encryption_key,
        memo.as_ref(),
        Some(aead_nonce),
    )
    .map_err(|e| OotleSdkError::Stealth(format!("AEAD encrypt failed: {e}")))?;

    let crypto = StealthCryptoApi::new();
    let auth = match output_spec.pay_to {
        StealthPayTo::StealthPublicKey => {
            let owner_public_key =
                crypto.derive_stealth_owner_public_key(internal_network, &account_key, &view_key, &nonce_secret);
            SpendAuthorization::Key(owner_public_key.to_byte_type())
        },
        // An `AccessRule::AllowAll` output is gated by a single-leaf condition tree; commit its root.
        StealthPayTo::AccessRuleAllowAll => SpendAuthorization::Script(
            condition_root(&[SpendCondition::access_rule(AccessRule::AllowAll)])
                .map_err(|e| OotleSdkError::Stealth(format!("compute allow-all condition root: {e}")))?,
        ),
    };

    // An explicit tag overrides the derived one; otherwise derive the UTXO scanning tag.
    let tag = match &output_spec.utxo_tag {
        Some(t) => UtxoTag::new(t.to_u32()),
        None => crypto.derive_stealth_output_tag(internal_network, &nonce_secret, &view_key, &resource_address),
    };

    Ok(StealthOutputWitness {
        witness: OutputWitness {
            amount: output_spec.amount,
            mask,
            sender_public_nonce: public_nonce,
            minimum_value_promise: output_spec.minimum_value_promise,
            encrypted_data,
            resource_view_key,
        },
        auth,
        tag,
    })
}

/// Builds the aggregated [`StealthOutputsStatement`] for every output in `intent`, plus the
/// aggregated output mask (the sum of the per-output masks), from an internally-expanded
/// [`StealthEntropy`].
///
/// `entropy` is produced by the KDF expansion ([`StealthEntropy::from_seed`]) — never host-supplied.
/// Calls **no** RNG itself; the aggregated bulletproof's internal randomness lives inside
/// `tari_crypto`. The viewable-balance proof, AEAD ciphertext, commitments, spend conditions and tags
/// **are** byte-reproducible (from the expanded entropy).
pub(crate) fn build_stealth_outputs_statement_from_entropy(
    network: Network,
    intent: &StealthTransferIntent,
    entropy: &StealthEntropy,
    revealed_receiver: Option<RistrettoPublicKeyBytes>,
) -> Result<(StealthOutputsStatement, SecretKeyBytes), OotleSdkError> {
    if intent.outputs.len() != entropy.per_output.len() {
        return Err(OotleSdkError::Validation(format!(
            "entropy.per_output.len() ({}) must equal intent.outputs.len() ({})",
            entropy.per_output.len(),
            intent.outputs.len()
        )));
    }

    // Build each witness; accumulate the aggregated output mask as a Ristretto scalar.
    let mut witnesses = Vec::with_capacity(intent.outputs.len());
    let mut agg_output_mask = RistrettoSecretKey::default();
    for (spec, per_entropy) in intent.outputs.iter().zip(entropy.per_output.iter()) {
        let witness = build_stealth_output_witness(network, spec, per_entropy)?;
        agg_output_mask = agg_output_mask + &witness.witness.mask;
        witnesses.push((witness, per_entropy));
    }

    // Assemble each StealthUnspentOutput, threading the injected ElGamal/ZK nonces for view-key
    // outputs through the seeded viewable-balance-proof seam.
    let mut outputs = Vec::with_capacity(witnesses.len());
    for (witness, per_entropy) in &witnesses {
        let unblinded = &witness.witness;
        let commitment = unblinded.to_commitment();

        let viewable_balance_proof = match &unblinded.resource_view_key {
            Some(view_key) => {
                let elgamal_nonce = per_entropy.elgamal_nonce.as_ref().ok_or_else(|| {
                    OotleSdkError::Validation(
                        "output has a resource_view_key but its entropy slice has no elgamal_nonce".to_string(),
                    )
                })?;
                let zk = per_entropy.zk_nonces.as_ref().ok_or_else(|| {
                    OotleSdkError::Validation(
                        "output has a resource_view_key but its entropy slice has no zk_nonces".to_string(),
                    )
                })?;
                let r = parse_secret(elgamal_nonce.as_bytes())?;
                let x_v = parse_secret(zk[0].as_bytes())?;
                let x_m = parse_secret(zk[1].as_bytes())?;
                let x_r = parse_secret(zk[2].as_bytes())?;
                let proof = generate_elgamal_viewable_balance_proof_seeded(
                    &unblinded.mask,
                    unblinded.amount,
                    &commitment,
                    view_key,
                    &r,
                    &x_v,
                    &x_m,
                    &x_r,
                )
                .map_err(|e| OotleSdkError::Stealth(format!("viewable balance proof failed: {e}")))?;
                Some(proof)
            },
            None => None,
        };

        outputs.push(StealthUnspentOutput {
            output: UnspentOutput {
                commitment: commitment.to_byte_type(),
                sender_public_nonce: unblinded.sender_public_nonce.to_byte_type(),
                encrypted_data: unblinded.encrypted_data.clone(),
                minimum_value_promise: unblinded.minimum_value_promise,
                viewable_balance_proof,
            },
            auth: witness.auth.clone(),
            tag: witness.tag,
        });
    }

    // Aggregated bulletproof over the output witnesses (stock random path).
    let agg_range_proof = generate_extended_bullet_proof(witnesses.iter().map(|(w, _)| &w.witness))
        .map_err(|e| OotleSdkError::Stealth(format!("range proof failed: {e}")))?;

    let revealed_output = match (intent.revealed_output_amount, revealed_receiver) {
        (0, _) => None,
        (amount, Some(receiver)) => Some(RevealedOutput::new(Amount::from_u64(amount), receiver)),
        (_, None) => {
            return Err(OotleSdkError::Validation(
                "a revealed output must name the receiver authorised to take it".to_string(),
            ));
        },
    };

    let statement = StealthOutputsStatement {
        outputs,
        revealed_output,
        agg_range_proof,
    };

    let agg_mask_bytes = SecretKeyBytes::from_bytes(agg_output_mask.as_bytes())
        .map_err(|e| OotleSdkError::Key(format!("aggregated mask is not 32 bytes: {e}")))?;

    Ok((statement, agg_mask_bytes))
}

/// The seed-reproducible entry point: expands `seed` into the per-output + bundle entropy, then builds
/// the statement. The viewable-balance proof, AEAD ciphertext, commitments, spend conditions and tags
/// are byte-reproducible from the seed; the aggregated bulletproof is not ([SC-BULLETPROOF]).
pub fn build_stealth_outputs_statement_with_seed(
    network: Network,
    intent: &StealthTransferIntent,
    seed: &BuildSeed,
    revealed_receiver: Option<RistrettoPublicKeyBytes>,
) -> Result<(StealthOutputsStatement, SecretKeyBytes), OotleSdkError> {
    seed.validate_nonzero()?;
    let entropy = StealthEntropy::from_seed(seed, intent.outputs.len());
    build_stealth_outputs_statement_from_entropy(network, intent, &entropy, revealed_receiver)
}

/// The random-nonce default entry point: expands a fresh OS-RNG seed, then builds the statement. The
/// resulting `agg_range_proof` (and thus the whole statement) is **not** reproducible — the safe
/// default for real submission.
pub fn build_stealth_outputs_statement(
    network: Network,
    intent: &StealthTransferIntent,
    revealed_receiver: Option<RistrettoPublicKeyBytes>,
) -> Result<(StealthOutputsStatement, SecretKeyBytes), OotleSdkError> {
    let entropy = StealthEntropy::from_os_rng(intent.outputs.len());
    build_stealth_outputs_statement_from_entropy(network, intent, &entropy, revealed_receiver)
}

#[cfg(test)]
mod tests {
    use tari_crypto::{
        keys::PublicKey as _,
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    };
    use tari_engine_types::stealth::validate_stealth_outputs_statement;

    use super::*;
    use crate::types::{
        address::ResourceAddressStr,
        bytes::PublicKeyBytes,
        numeric::BoundaryAmount,
        stealth::{PerOutputEntropy, StealthEntropy, StealthInputSpec},
    };

    fn pk_bytes(seed: u8) -> PublicKeyBytes {
        let sk = RistrettoSecretKey::from_canonical_bytes(&{
            let mut b = [0u8; 32];
            b[0] = seed;
            b
        })
        .unwrap();
        let pk = RistrettoPublicKey::from_secret_key(&sk);
        PublicKeyBytes::from_bytes(pk.as_bytes()).unwrap()
    }

    fn secret(seed: u8) -> SecretKeyBytes {
        let mut b = [0u8; 32];
        b[0] = seed;
        // Assert canonical so a bad seed fails loudly.
        RistrettoSecretKey::from_canonical_bytes(&b).expect("canonical low scalar");
        SecretKeyBytes::from_array(b)
    }

    fn tari_resource() -> ResourceAddressStr {
        ResourceAddressStr::parse(tari_template_lib_types::constants::STEALTH_TARI_RESOURCE_ADDRESS.to_string())
            .unwrap()
    }

    fn output_spec(amount: u64, with_view_key: bool) -> StealthOutputSpec {
        StealthOutputSpec {
            destination_account_pk: pk_bytes(3),
            destination_view_pk: pk_bytes(4),
            amount,
            revealed_amount: 0,
            resource_address: tari_resource(),
            resource_view_key: with_view_key.then(|| pk_bytes(5)),
            memo: None,
            pay_to: StealthPayTo::StealthPublicKey,
            utxo_tag: None,
            minimum_value_promise: 0,
        }
    }

    fn per_entropy(base: u8, with_view: bool) -> PerOutputEntropy {
        PerOutputEntropy {
            mask: secret(base),
            sender_nonce: secret(base + 1),
            aead_nonce: secret(base + 2),
            elgamal_nonce: with_view.then(|| secret(base + 3)),
            zk_nonces: with_view.then(|| [secret(base + 4), secret(base + 5), secret(base + 6)]),
        }
    }

    fn entropy_for(specs: &[(u8, bool)]) -> StealthEntropy {
        StealthEntropy {
            per_output: specs.iter().map(|(b, v)| per_entropy(*b, *v)).collect(),
        }
    }

    fn intent(outputs: Vec<StealthOutputSpec>, revealed_output: u64) -> StealthTransferIntent {
        StealthTransferIntent {
            from_account: crate::types::address::ComponentAddressStr::parse(
                tari_template_lib_types::ComponentAddress::new(tari_template_lib_types::ObjectKey::from_array(
                    [0xaa; tari_template_lib_types::ObjectKey::LENGTH],
                ))
                .to_string(),
            )
            .unwrap(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: Vec::<StealthInputSpec>::new(),
            outputs,
            revealed_input_amount: 0,
            revealed_output_amount: revealed_output,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        }
    }

    // (a) Reproducibility (deterministic path): the byte-reproducible fields match across two runs.
    // The agg_range_proof is NOT byte-stable, so we compare every other field.
    #[test]
    fn deterministic_path_is_reproducible_except_bulletproof() {
        let it = intent(vec![output_spec(1000, false)], 0);
        let e = entropy_for(&[(10, false)]);
        let (s1, m1) = build_stealth_outputs_statement_from_entropy(Network::LocalNet, &it, &e, None).unwrap();
        let (s2, m2) = build_stealth_outputs_statement_from_entropy(Network::LocalNet, &it, &e, None).unwrap();
        assert_eq!(m1, m2, "aggregated mask must be reproducible");
        assert_eq!(
            s1.outputs, s2.outputs,
            "deterministic output fields must be reproducible"
        );
        assert_eq!(s1.revealed_output, s2.revealed_output);
        // The bulletproof is expected to differ — both are still valid proofs.
        validate_stealth_outputs_statement(&s1, None).unwrap();
        validate_stealth_outputs_statement(&s2, None).unwrap();
    }

    // (b) validate_stealth_outputs_statement accepts the built statement.
    #[test]
    fn built_statement_validates() {
        let it = intent(vec![output_spec(5000, false)], 0);
        let e = entropy_for(&[(20, false)]);
        let (stmt, _) = build_stealth_outputs_statement_from_entropy(Network::LocalNet, &it, &e, None).unwrap();
        validate_stealth_outputs_statement(&stmt, None).unwrap();
        assert_eq!(stmt.outputs.len(), 1);
    }

    // (c) minimum_value_promise > amount rejected.
    #[test]
    fn rejects_minimum_value_promise_above_amount() {
        let mut spec = output_spec(100, false);
        spec.minimum_value_promise = 101;
        let it = intent(vec![spec], 0);
        let e = entropy_for(&[(30, false)]);
        let err = build_stealth_outputs_statement_from_entropy(Network::LocalNet, &it, &e, None).unwrap_err();
        assert!(matches!(err, OotleSdkError::Validation(_)));
    }

    // (d) output count vs entropy count mismatch rejected.
    #[test]
    fn rejects_output_entropy_count_mismatch() {
        let it = intent(vec![output_spec(100, false), output_spec(200, false)], 0);
        let e = entropy_for(&[(40, false)]); // only one slice for two outputs
        let err = build_stealth_outputs_statement_from_entropy(Network::LocalNet, &it, &e, None).unwrap_err();
        assert!(matches!(err, OotleSdkError::Validation(_)));
    }

    // (e) production path produces a valid statement (no byte assertion).
    #[test]
    fn production_path_validates() {
        let it = intent(vec![output_spec(7777, false)], 0);
        let (stmt, _) = build_stealth_outputs_statement(Network::LocalNet, &it, None).unwrap();
        validate_stealth_outputs_statement(&stmt, None).unwrap();
    }

    // (f) view-key output includes a viewable_balance_proof.
    #[test]
    fn view_key_output_has_viewable_balance_proof() {
        let it = intent(vec![output_spec(9000, true)], 0);
        let e = entropy_for(&[(50, true)]);
        let (stmt, _) = build_stealth_outputs_statement_from_entropy(Network::LocalNet, &it, &e, None).unwrap();
        assert!(stmt.outputs[0].output.viewable_balance_proof.is_some());
        // The view key validation path accepts it.
        let view_key = RistrettoPublicKey::from_canonical_bytes(pk_bytes(5).as_bytes()).unwrap();
        validate_stealth_outputs_statement(&stmt, Some(&view_key)).unwrap();
    }

    // A view-key output whose entropy slice omits the ZK nonces is a Validation error.
    #[test]
    fn view_key_output_without_zk_nonces_is_rejected() {
        let it = intent(vec![output_spec(9000, true)], 0);
        let e = entropy_for(&[(50, false)]); // no elgamal/zk nonces
        let err = build_stealth_outputs_statement_from_entropy(Network::LocalNet, &it, &e, None).unwrap_err();
        assert!(matches!(err, OotleSdkError::Validation(_)));
    }

    // (g) no-output (revealed-only) case: empty outputs + empty range proof.
    #[test]
    fn no_output_revealed_only() {
        let it = intent(vec![], 1_000_000);
        let e = entropy_for(&[]);
        let (stmt, mask) =
            build_stealth_outputs_statement_from_entropy(Network::LocalNet, &it, &e, Some(a_receiver())).unwrap();
        assert!(stmt.outputs.is_empty());
        assert!(stmt.agg_range_proof.is_empty());
        assert_eq!(mask, SecretKeyBytes::from_array([0u8; 32]));
        validate_stealth_outputs_statement(&stmt, None).unwrap();
    }

    /// A stand-in receiver for a revealed output. These tests build the outputs statement directly, so nothing here
    /// checks the key against a signer; that check is the engine's.
    fn a_receiver() -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(
            tari_crypto::ristretto::RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(7u64)).as_bytes(),
        )
        .unwrap()
    }

    // A revealed output with no receiver has no key authorised to take it, so it is refused at build time rather
    // than emitted for the engine to reject.
    #[test]
    fn revealed_output_without_a_receiver_is_rejected() {
        let it = intent(vec![], 1_000_000);
        let e = entropy_for(&[]);
        let err = build_stealth_outputs_statement_from_entropy(Network::LocalNet, &it, &e, None).unwrap_err();
        assert!(matches!(err, OotleSdkError::Validation(_)), "unexpected error: {err}");
    }

    // The witness build is decryptable by the recipient (sanity that the AEAD-injected path is correct).
    #[test]
    fn witness_is_decryptable_with_injected_nonce() {
        use tari_crypto::commitment::HomomorphicCommitmentFactory;
        use tari_engine_types::crypto::get_commitment_factory;

        let amount = 12_345u64;
        // Use known account/view secret keys so we can decrypt.
        let view_sk = RistrettoSecretKey::from_canonical_bytes(&{
            let mut b = [0u8; 32];
            b[0] = 4;
            b
        })
        .unwrap();
        let spec = output_spec(amount, false);
        let pe = per_entropy(60, false);
        let witness = build_stealth_output_witness(Network::LocalNet, &spec, &pe).unwrap();

        let commitment = get_commitment_factory()
            .commit_value(&witness.witness.mask, amount)
            .to_byte_type();
        let encryption_key = kdfs::encrypted_data_dh_kdf_aead(&view_sk, &witness.witness.sender_public_nonce);
        let decrypted = tari_ootle_wallet_crypto::encrypted_data::unblind_output(
            &commitment,
            &witness.witness.encrypted_data,
            &encryption_key,
            false,
        )
        .unwrap();
        assert_eq!(decrypted.value(), amount);
        assert_eq!(decrypted.mask().as_bytes(), witness.witness.mask.as_bytes());
    }

    // --- Seed entry-point: anti-leak + reproducibility + rails -----------------------------------

    /// A multi-output view-key build derives a **distinct per-output `x_m`** for each output (the
    /// commitment-mask ZK nonce). Cross-output reuse — the leak path `mask = (s_m − s_m')/(e − e')` —
    /// is therefore structurally impossible: the two outputs' masks are independent seed derivations
    /// bound to the output index.
    #[test]
    fn two_view_key_outputs_use_distinct_per_output_zk_nonces() {
        let seed = BuildSeed::from_array([0x5e; 32]);
        let entropy = StealthEntropy::from_seed(&seed, 2);
        let zk0 = entropy.per_output[0].zk_nonces.as_ref().unwrap();
        let zk1 = entropy.per_output[1].zk_nonces.as_ref().unwrap();
        // x_m is index 1 of [x_v, x_m, x_r].
        assert_ne!(zk0[1], zk1[1], "per-output x_m must differ across outputs");
        // Index-swap changes them: an x_m for output 0 is never an x_m for output 1.
        assert_ne!(zk0[1], zk1[1]);
        // Every flattened scalar across both outputs is distinct (the rail asserts this too).
        entropy.validate().expect("seed-derived entropy is pairwise-distinct");
    }

    /// Same seed + same intent ⇒ identical statement for every byte-stable field (the aggregated
    /// bulletproof is excluded per SC-BULLETPROOF).
    #[test]
    fn with_seed_is_reproducible_modulo_bulletproof() {
        let seed = BuildSeed::from_array([0x77; 32]);
        let it = intent(vec![output_spec(1_000_000, true)], 0);
        let (a, ma) = build_stealth_outputs_statement_with_seed(Network::LocalNet, &it, &seed, None).unwrap();
        let (b, mb) = build_stealth_outputs_statement_with_seed(Network::LocalNet, &it, &seed, None).unwrap();
        assert_eq!(ma, mb, "aggregated output mask is reproducible");
        assert_eq!(
            a.outputs, b.outputs,
            "every output field (incl. the viewable proof) is reproducible"
        );
        assert_eq!(a.revealed_output, b.revealed_output);
        // The aggregated bulletproof is the only non-byte-stable field — deliberately not compared.
    }

    /// An all-zero seed is rejected by the rail before any derivation runs.
    #[test]
    fn with_seed_zero_seed_is_a_validation_error() {
        let it = intent(vec![output_spec(1_000_000, false)], 0);
        let err =
            build_stealth_outputs_statement_with_seed(Network::LocalNet, &it, &BuildSeed::from_array([0u8; 32]), None)
                .unwrap_err();
        assert_eq!(err.code(), "VALIDATION");
    }
}
