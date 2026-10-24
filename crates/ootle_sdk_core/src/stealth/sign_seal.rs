//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Stealth signing + sealing + encoding — the send-side capstone. A pure, synchronous stage driven
//! by the [`StealthSignatureRequirementsState`] the [`StealthPartialTransaction`] carries. The three
//! seal cases are:
//!
//! 1. **Account-key seal** (`must_sign_with_account_key == true`): seal with the caller-supplied account secret; emit
//!    one stealth-key (`c+k`) authorization signature per `other_signers` entry.
//! 2. **Stealth seal** (`!must_sign_with_account_key && seal_signer.is_some()`, or the first input promoted to seal
//!    signer): seal with the one-time key `c+k` (`c = H(v·R)`) derived from the seal input's `public_nonce` via
//!    [`owner_stealth_dh_secret`](tari_ootle_wallet_crypto::kdfs::owner_stealth_dh_secret).
//! 3. **Ephemeral seal** (`can_sign_with_ephemeral_key()`): seal with a fresh random one-time ephemeral key.
//!
//! Every signature (auth + seal) is signed with a **fresh random Schnorr nonce** ([`sign_random`]) over
//! the `create_message_v1` digests, and [`crate::tx::bor_encode`] encodes the result. The
//! `main_signer` is set to `SealSigner` before building the unsealed tx in every seal case: all three
//! seal with a key the spender controls, so the seal signer is always an authority here. (The public
//! path in [`crate::tx`] assigns it from the authorization-signature count instead, because a public
//! transaction's seal signer need not be one.)
//!
//! ## Comparison mode
//!
//! The seal/authorization signatures are drawn with random nonces (so their scalars, and thus the
//! sealed bytes and id, are not reproducible), and the embedded aggregated bulletproof / balance-proof
//! signature are likewise byte-unstable. So the full send vectors compare semantically: the sealed
//! transaction is re-decoded, its signatures re-verified, and its deterministic decoded fields
//! compared, rather than asserting byte equality. The seal-signer **public key** is deterministic
//! (`c+k` derives from the account and view secrets + the input nonce; the account key seals case (i)), so the
//! key-selection contract is still locked.

use ootle_network::Network as InternalNetwork;
use tari_crypto::{
    keys::SecretKey,
    ristretto::{RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_ootle_transaction::{
    Transaction,
    TransactionId,
    TransactionSealSignature,
    TransactionSignature,
    UnsealedTransactionV1,
    UnsignedTransaction,
    UnsignedTransactionV1,
};
use tari_ootle_wallet_crypto::kdfs::owner_stealth_dh_secret;
use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

use crate::{
    inputs::{FetchedSubstate, StealthSignerEntry},
    keys::public_key_bytes_from_secret,
    public_transfer::EncodedPublicTransfer,
    stealth::{
        assemble::build_stealth_transfer_unsigned_with_seed,
        partial::{StealthPartialTransaction, StealthSignatureRequirementsState},
    },
    tx::bor_encode,
    types::{
        bytes::{BuildSeed, EncodedTransactionBytes, SecretKeyBytes, TransactionIdBytes},
        error::OotleSdkError,
        network::Network,
        stealth::{StealthTransferIntent, random_seed},
    },
};

/// The caller-supplied secret bundle a stealth seal consumes.
///
/// The account secret is required for **all three** seal cases: case (i) seals with it directly;
/// cases (ii)/(iii) derive the `c+k` one-time keys / authorize stealth signers from it. The view
/// secret keys the one-time tweak `c = H(v·R)`.
#[derive(Debug, Clone)]
pub struct StealthKeys {
    /// Account secret key (seals directly in case i; derives `c+k` in cases ii/iii).
    pub account_secret: SecretKeyBytes,
    /// View secret key of the same account, deriving the tweak `c` of each one-time key.
    pub view_secret: SecretKeyBytes,
}

impl StealthKeys {
    /// Builds the bundle from the account and view secrets.
    pub fn new(account_secret: SecretKeyBytes, view_secret: SecretKeyBytes) -> Self {
        Self {
            account_secret,
            view_secret,
        }
    }
}

/// The parsed account and view secrets a stealth spend derives its one-time keys from.
pub(crate) struct SpendSecrets {
    account: RistrettoSecretKey,
    view: RistrettoSecretKey,
}

impl SpendSecrets {
    pub(crate) fn parse(keys: &StealthKeys) -> Result<Self, OotleSdkError> {
        Ok(Self {
            account: parse_secret(&keys.account_secret)?,
            view: parse_secret(&keys.view_secret)?,
        })
    }
}

/// Signs a 64-byte message with a fresh random Schnorr nonce and converts to the byte-typed signature.
fn sign_random(secret: &RistrettoSecretKey, message: &[u8; 64]) -> Result<SchnorrSignatureBytes, OotleSdkError> {
    use ootle_byte_type::ToByteType;
    let sig = RistrettoSchnorr::sign(secret, *message, &mut rand::rng())
        .map_err(|e| OotleSdkError::Key(format!("stealth signing failed: {e}")))?;
    Ok(sig.to_byte_type())
}

/// Parses a boundary secret scalar (account or view secret) into the internal scalar.
fn parse_secret(bytes: &SecretKeyBytes) -> Result<RistrettoSecretKey, OotleSdkError> {
    RistrettoSecretKey::from_canonical_bytes(bytes.as_bytes())
        .map_err(|e| OotleSdkError::Key(format!("invalid secret key: {e}")))
}

/// Recovers the internal `RistrettoPublicKey` for a signer's recorded sender public nonce.
fn parse_signer_public_nonce(signer: &StealthSignerEntry) -> Result<RistrettoPublicKey, OotleSdkError> {
    RistrettoPublicKey::from_canonical_bytes(signer.public_nonce_bytes.as_bytes())
        .map_err(|e| OotleSdkError::Key(format!("invalid signer public nonce: {e}")))
}

/// Derives a signer's one-time spend secret `c+k` from the account and view secrets and the signer's
/// sender public nonce, via the reused KDF leaf. `network` is always the transfer's network — different
/// networks yield different stealth keys.
fn derive_stealth_secret(
    network: InternalNetwork,
    secrets: &SpendSecrets,
    signer: &StealthSignerEntry,
) -> Result<RistrettoSecretKey, OotleSdkError> {
    let public_nonce = parse_signer_public_nonce(signer)?;
    Ok(owner_stealth_dh_secret(
        network,
        &secrets.account,
        &secrets.view,
        &public_nonce,
    ))
}

/// Builds one stealth-key (`c+k`) authorization [`TransactionSignature`] for `signer`.
///
/// The message commits to the **seal signer's** public key (`create_message_v1`'s `SealSigner`
/// field); the signature is made with the signer's derived `c+k` secret and a random nonce; the
/// verifying key recorded is `(c+k)·G`.
fn build_stealth_auth_signature(
    network: InternalNetwork,
    secrets: &SpendSecrets,
    signer: &StealthSignerEntry,
    seal_signer_pk: &RistrettoPublicKeyBytes,
    unsigned_v1: &UnsignedTransactionV1,
) -> Result<TransactionSignature, OotleSdkError> {
    let stealth_secret = derive_stealth_secret(network, secrets, signer)?;
    let stealth_pk = public_key_bytes_from_secret(&stealth_secret);
    let message = TransactionSignature::create_message_v1(seal_signer_pk, unsigned_v1);
    let sig_bytes = sign_random(&stealth_secret, &message)?;
    Ok(TransactionSignature::new(stealth_pk, sig_bytes))
}

/// The seal key selected for a given signature-requirements case, plus the authorization signatures to
/// attach (in order) before sealing. The seal signature itself is drawn (random nonce) in
/// [`finalize_seal`].
struct SealPlan {
    /// The secret that seals the transaction.
    seal_secret: RistrettoSecretKey,
    /// The authorization signatures to attach (in order) before sealing.
    auth_signatures: Vec<TransactionSignature>,
}

/// Resolves the seal key and authorization signatures for the given case. Every signature is made with
/// a fresh random nonce; the ephemeral case draws a fresh random seal key.
/// The secret the transfer will seal with, or `None` for the ephemeral case, which draws a fresh key at seal time.
///
/// Assembly needs the matching public key before the balance proof is signed — a revealed output names the key
/// authorised to take it, and that key must be one the transaction actually carries — while [`plan_seal`] needs the
/// secret itself at seal time. Both go through here so the receiver named in the statement is the key that seals.
fn seal_secret(
    network: InternalNetwork,
    secrets: &SpendSecrets,
    sig_reqs: &StealthSignatureRequirementsState,
) -> Result<Option<RistrettoSecretKey>, OotleSdkError> {
    if sig_reqs.must_sign_with_account_key {
        return Ok(Some(secrets.account.clone()));
    }
    effective_seal_signer(sig_reqs)
        .map(|signer| derive_stealth_secret(network, secrets, signer))
        .transpose()
}

/// The public key that will seal a stealth transfer assembled from `sig_reqs`, or `None` when the seal key is the
/// ephemeral one drawn at seal time. See [`seal_secret`].
pub(crate) fn seal_public_key(
    network: Network,
    secrets: &SpendSecrets,
    sig_reqs: &StealthSignatureRequirementsState,
) -> Result<Option<RistrettoPublicKeyBytes>, OotleSdkError> {
    Ok(seal_secret(network.into(), secrets, sig_reqs)?.map(|secret| public_key_bytes_from_secret(&secret)))
}

fn plan_seal(
    network: InternalNetwork,
    secrets: &SpendSecrets,
    sig_reqs: &StealthSignatureRequirementsState,
    unsigned_v1: &UnsignedTransactionV1,
) -> Result<SealPlan, OotleSdkError> {
    let account_secret = &secrets.account;
    if sig_reqs.must_sign_with_account_key {
        // Case (i): account key seals; account-key auth sig + stealth-key auth sigs for other signers.
        let seal_pk = public_key_bytes_from_secret(account_secret);
        let mut auth_signatures = Vec::with_capacity(sig_reqs.other_signers.len() + 1);

        // The account-key authorization signature.
        let account_auth_msg = TransactionSignature::create_message_v1(&seal_pk, unsigned_v1);
        let account_auth_sig = sign_random(account_secret, &account_auth_msg)?;
        auth_signatures.push(TransactionSignature::new(seal_pk, account_auth_sig));

        // One stealth-key authorization signature per other signer.
        for signer in sig_reqs.other_signers_iter() {
            auth_signatures.push(build_stealth_auth_signature(
                network,
                secrets,
                signer,
                &seal_pk,
                unsigned_v1,
            )?);
        }

        Ok(SealPlan {
            seal_secret: account_secret.clone(),
            auth_signatures,
        })
    } else if let Some(seal_secret) = seal_secret(network, secrets, sig_reqs)? {
        // Case (ii): a stealth input seals with its one-time key c+k.
        let seal_pk = public_key_bytes_from_secret(&seal_secret);

        // Authorization signatures for the remaining required signers (the seal signer is skipped by
        // `other_signers_iter` when it was the promoted first input).
        let mut auth_signatures = Vec::new();
        for signer in sig_reqs.other_signers_iter() {
            auth_signatures.push(build_stealth_auth_signature(
                network,
                secrets,
                signer,
                &seal_pk,
                unsigned_v1,
            )?);
        }

        Ok(SealPlan {
            seal_secret,
            auth_signatures,
        })
    } else {
        // Case (iii): ephemeral key seal (no inputs to spend). A fresh random one-time key; no
        // authorization signatures.
        let seal_secret = RistrettoSecretKey::random(&mut rand::rng());
        Ok(SealPlan {
            seal_secret,
            auth_signatures: Vec::new(),
        })
    }
}

/// The seal signer for the stealth-seal case: the explicit `seal_signer`, else the first required
/// signer promoted to seal signer. Returns `None` for the account-key case (the account key seals,
/// not a stealth signer) and for the ephemeral case (no signers at all). The
/// `must_sign_with_account_key` short-circuit makes the case-(i) invariant self-enforcing even if a
/// future caller invokes this before the dispatch guard.
fn effective_seal_signer(sig_reqs: &StealthSignatureRequirementsState) -> Option<&StealthSignerEntry> {
    if sig_reqs.must_sign_with_account_key {
        return None;
    }
    sig_reqs.seal_signer.as_ref().or_else(|| sig_reqs.other_signers.first())
}

/// Seals + encodes an already-assembled [`StealthPartialTransaction`]. Every signature is drawn with a
/// fresh random nonce, so the bytes/id are **not** reproducible (the safe default for real submission).
/// The seal case is driven by the partial's [`StealthSignatureRequirementsState`].
pub fn seal_and_encode_stealth_transfer(
    network: Network,
    partial: StealthPartialTransaction,
    keys: &StealthKeys,
) -> Result<EncodedPublicTransfer, OotleSdkError> {
    let internal_network: InternalNetwork = network.into();
    let secrets = SpendSecrets::parse(keys)?;

    let (unsigned, sig_reqs) = partial.into_parts();
    let UnsignedTransaction::V1(mut unsigned_v1) = unsigned;

    let plan = plan_seal(internal_network, &secrets, &sig_reqs, &unsigned_v1)?;

    let transaction = finalize_seal(&mut unsigned_v1, plan)?;
    let id = transaction.calculate_id();
    encode(transaction, id)
}

/// Sets `main_signer`, builds the unsealed tx with the planned authorization signatures,
/// computes the seal signature over the seal digest with the planned key (random nonce), and injects it.
fn finalize_seal(unsigned_v1: &mut UnsignedTransactionV1, plan: SealPlan) -> Result<Transaction, OotleSdkError> {
    // The main signer is committed by the seal digest, so it must be settled before constructing the
    // unsealed tx. Every seal case here seals with a key the spender controls — the account key, a
    // stealth input's one-time `c+k`, or the ephemeral key — so the seal signer is always an
    // authority, whatever authorization signatures accompany it. `SealSigner` is what puts its badge in
    // the transaction's auth scope, which a promoted stealth seal signer (which carries no
    // authorization signature of its own) needs to spend its own input, and which a revealed output
    // naming that key needs to be taken.
    unsigned_v1.main_signer = tari_ootle_transaction::MainSigner::SealSigner;

    let seal_pk = public_key_bytes_from_secret(&plan.seal_secret);
    let unsealed = UnsealedTransactionV1::new(unsigned_v1.clone(), plan.auth_signatures);

    let seal_message = TransactionSealSignature::create_message_v1(&unsealed);
    let seal_sig_bytes = sign_random(&plan.seal_secret, &seal_message)?;
    let seal_sig = TransactionSealSignature::new(seal_pk, seal_sig_bytes);

    Ok(unsealed.seal_with_signature(seal_sig))
}

/// Wraps the sealed transaction + id into the boundary [`EncodedPublicTransfer`] (encoded bytes + id,
/// both lowercase hex).
fn encode(transaction: Transaction, id: TransactionId) -> Result<EncodedPublicTransfer, OotleSdkError> {
    let bytes = bor_encode(transaction)?;
    Ok(EncodedPublicTransfer {
        encoded_transaction: EncodedTransactionBytes::from_vec(bytes),
        transaction_id: TransactionIdBytes::from_bytes(id.as_bytes()),
    })
}

/// Full-pipeline seed-reproducible **construction** send: build (from `seed`) → seal/encode. The
/// commitments/masks/ciphertext are reproducible from the seed; the signatures are random.
///
/// `fetched_utxos` are every stealth-input UTXO substate, fetched up front; `spend_secrets` are the
/// per-input view-only secrets (one per `intent.inputs`, positional) used to decrypt the input masks.
pub fn build_and_encode_stealth_transfer_with_seed(
    network: Network,
    intent: &StealthTransferIntent,
    fetched_utxos: &[FetchedSubstate],
    spend_secrets: &[SecretKeyBytes],
    keys: &StealthKeys,
    seed: &BuildSeed,
) -> Result<EncodedPublicTransfer, OotleSdkError> {
    let partial = build_stealth_transfer_unsigned_with_seed(network, intent, fetched_utxos, spend_secrets, keys, seed)?;
    seal_and_encode_stealth_transfer(network, partial, keys)
}

/// Full-pipeline random-construction send: expands the proof + construction entropy from a fresh
/// OS-RNG seed, then builds + seals. Nothing is reproducible (the safe default for real submission).
pub fn build_and_encode_stealth_transfer(
    network: Network,
    intent: &StealthTransferIntent,
    fetched_utxos: &[FetchedSubstate],
    spend_secrets: &[SecretKeyBytes],
    keys: &StealthKeys,
) -> Result<EncodedPublicTransfer, OotleSdkError> {
    let seed = random_seed();
    let partial =
        build_stealth_transfer_unsigned_with_seed(network, intent, fetched_utxos, spend_secrets, keys, &seed)?;
    seal_and_encode_stealth_transfer(network, partial, keys)
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{keys::PublicKey as _, ristretto::RistrettoSecretKey};
    use tari_engine_types::{
        Utxo,
        UtxoOutput,
        crypto::{OutputBody, commit_u64_amount},
        substate::SubstateValue,
    };
    use tari_ootle_transaction::TransactionEnvelope;
    use tari_ootle_wallet_crypto::{encrypted_data::encrypt_data, kdfs, stealth::condition_root};
    use tari_template_lib_types::{
        ComponentAddress,
        ObjectKey,
        access_rules::AccessRule,
        crypto::UtxoTag,
        stealth::{SpendAuthorization, SpendCondition},
    };

    use super::*;
    use crate::{
        inputs::FetchedSubstate,
        stealth::inputs::stealth_utxo_substate_id,
        tx::verify_all_signatures,
        types::{
            address::{ComponentAddressStr, ResourceAddressStr},
            bytes::PublicKeyBytes,
            numeric::BoundaryAmount,
            stealth::{StealthInputSpec, StealthOutputSpec, StealthPayTo},
        },
    };

    const NETWORK: Network = Network::LocalNet;

    fn fixed_scalar(seed: u8) -> RistrettoSecretKey {
        let mut b = [0u8; 32];
        b[0] = seed;
        RistrettoSecretKey::from_canonical_bytes(&b).expect("low scalar is canonical")
    }

    fn secret(seed: u8) -> SecretKeyBytes {
        SecretKeyBytes::from_bytes(fixed_scalar(seed).as_bytes()).unwrap()
    }

    /// A fixed build seed for the seed-reproducible build paths.
    fn build_seed(byte: u8) -> BuildSeed {
        BuildSeed::from_array([byte; 32])
    }

    fn pk_bytes(seed: u8) -> PublicKeyBytes {
        let pk = RistrettoPublicKey::from_secret_key(&fixed_scalar(seed));
        PublicKeyBytes::from_bytes(pk.as_bytes()).unwrap()
    }

    fn tari_resource() -> ResourceAddressStr {
        ResourceAddressStr::parse(tari_template_lib_types::constants::STEALTH_TARI_RESOURCE_ADDRESS.to_string())
            .unwrap()
    }

    fn from_component() -> ComponentAddressStr {
        ComponentAddressStr::parse(ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH])).to_string())
            .unwrap()
    }

    fn output_spec(amount: u64) -> StealthOutputSpec {
        StealthOutputSpec {
            destination_account_pk: pk_bytes(3),
            destination_view_pk: pk_bytes(4),
            amount,
            revealed_amount: 0,
            resource_address: tari_resource(),
            resource_view_key: None,
            memo: None,
            pay_to: StealthPayTo::StealthPublicKey,
            utxo_tag: None,
            minimum_value_promise: 0,
        }
    }

    /// The view secret the stealth-seal input is encrypted to (see [`stealth_seal_partial`]).
    const VIEW_SECRET_SEED: u8 = 162;

    fn stealth_keys() -> StealthKeys {
        StealthKeys::new(secret(11), secret(VIEW_SECRET_SEED))
    }

    /// Builds an ephemeral-case (iii) partial.
    ///
    /// # Why the ephemeral case is cryptographically unreachable in a balanced transfer
    ///
    /// The genuine ephemeral case (`can_sign_with_ephemeral_key()`) requires `must_sign_with_account_key
    /// == false` (⇒ `revealed_input_amount == 0`) **and** `seal_signer.is_none() && other_signers
    /// .is_empty()` (⇒ no stealth inputs). That shape — `inputs_statement.revealed_amount == 0` **and**
    /// `inputs_statement.inputs.is_empty()` — is **categorically rejected** by the engine's
    /// `validate_transfer` pre-flight (called before emitting any partial):
    ///
    /// ```text
    /// if transfer.inputs_statement.revealed_amount.is_zero() && transfer.inputs_statement.inputs.is_empty() {
    ///     return Err(InvalidBalanceProof { details: "No inputs or revealed inputs provided" });
    /// }
    /// ```
    ///
    /// This is a **cryptographic invariant, not an accidental gap**: with no stealth inputs and no
    /// stealth outputs the public excess is `0·G`, so the balance signature `r + e·0 = r` would leak the
    /// signing nonce. There is therefore **no balanced transaction shape that triggers the ephemeral case
    /// yet passes assembly**, so the ephemeral seal is covered only by the unit tests below.
    ///
    /// ## How this helper exercises the path
    ///
    /// We take a valid, fully-assembled unsigned transaction (the account-key intent, which assembles +
    /// validates) and pair it with a hand-built ephemeral `StealthSignatureRequirementsState`.
    /// `seal_and_encode` only consumes the sig-reqs + the unsigned tx (it does not re-validate), so this
    /// faithfully exercises the case-(iii) seal path against a real, balanced transaction body.
    fn ephemeral_partial() -> StealthPartialTransaction {
        let (unsigned, _sig_reqs) = account_key_partial().into_parts();
        let ephemeral_reqs = StealthSignatureRequirementsState {
            must_sign_with_account_key: false,
            seal_signer: None,
            other_signers: Vec::new(),
        };
        assert!(
            ephemeral_reqs.can_sign_with_ephemeral_key(),
            "the hand-built sig-reqs must be the ephemeral case"
        );
        StealthPartialTransaction {
            unsigned,
            sig_reqs: ephemeral_reqs,
        }
    }

    /// The build seed the stealth send tests expand into proof + construction entropy.
    fn send_seed() -> BuildSeed {
        build_seed(0x90)
    }

    /// Builds a stealth-seal (case ii) partial from a fabricated, decryptable stealth UTXO input
    /// balanced against an equal-value output. Returns the partial + the (public_nonce,
    /// owner_account_secret) so the test can reconstruct the expected c+k seal key.
    fn stealth_seal_partial() -> (
        StealthPartialTransaction,
        RistrettoSecretKey,
        RistrettoPublicKey,
        BuildSeed,
    ) {
        let value = 1_000_000u64;
        let seed = send_seed();

        let input_mask = fixed_scalar(160);
        let nonce_secret = fixed_scalar(161);
        let public_nonce = RistrettoPublicKey::from_secret_key(&nonce_secret);
        let view_secret = fixed_scalar(VIEW_SECRET_SEED);

        let commitment = commit_u64_amount(&input_mask, value).to_byte_type();
        let commitment_hex = hex::encode(commitment.as_bytes());

        let encryption_key = kdfs::encrypted_data_dh_kdf_aead(&view_secret, &public_nonce);
        let encrypted_data = encrypt_data(value, &input_mask, &encryption_key, None).unwrap();
        let utxo = Utxo::new(UtxoOutput {
            output: OutputBody {
                public_nonce: public_nonce.to_byte_type(),
                encrypted_data,
                minimum_value_promise: 0,
                viewable_balance: None,
            },
            auth: SpendAuthorization::Script(
                condition_root(&[SpendCondition::access_rule(AccessRule::AllowAll)]).unwrap(),
            ),
            tag: UtxoTag::new(0),
        });

        // The owner account secret: we use the test account secret (so c+k = H(net, v·R) + acct).
        let owner_account_secret = fixed_scalar(11);
        let owner_pk = RistrettoPublicKey::from_secret_key(&owner_account_secret);
        let owner_pk_bytes = PublicKeyBytes::from_bytes(owner_pk.as_bytes()).unwrap();

        let substate_id = stealth_utxo_substate_id(tari_resource().as_str(), &commitment_hex).unwrap();
        let fetched = vec![FetchedSubstate {
            substate_id: substate_id.to_string(),
            version: 0,
            substate_value: crate::substate_decode::indexer_substate_json(&SubstateValue::Utxo(utxo)),
        }];

        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: vec![StealthInputSpec {
                commitment: crate::types::stealth::CommitmentBytes::from_hex(&commitment_hex).unwrap(),
                owner_account_pk: owner_pk_bytes,
            }],
            outputs: vec![output_spec(value)],
            revealed_input_amount: 0,
            revealed_output_amount: 0,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        };

        let spend_secrets = vec![SecretKeyBytes::from_bytes(view_secret.as_bytes()).unwrap()];
        let partial = build_stealth_transfer_unsigned_with_seed(
            NETWORK,
            &intent,
            &fetched,
            &spend_secrets,
            &stealth_keys(),
            &seed,
        )
        .unwrap();
        (partial, owner_account_secret, public_nonce, seed)
    }

    /// Builds an account-key-seal (case i) partial: a stealth output funded by a revealed-input
    /// bucket (`revealed_input_amount > 0` ⇒ the account key must sign).
    fn account_key_partial() -> StealthPartialTransaction {
        let intent = StealthTransferIntent {
            from_account: from_component(),
            resource_address: tari_resource(),
            fee: BoundaryAmount::new(2000),
            inputs: vec![],
            outputs: vec![output_spec(1_000_000)],
            revealed_input_amount: 1_000_000,
            revealed_output_amount: 0,
            min_epoch: None,
            max_epoch: 1,
            dry_run: false,
            pay_fee_from_revealed: false,
        };
        build_stealth_transfer_unsigned_with_seed(NETWORK, &intent, &[], &[], &stealth_keys(), &send_seed()).unwrap()
    }

    fn decode(out: &EncodedPublicTransfer) -> Transaction {
        TransactionEnvelope::from_raw(out.encoded_transaction.as_bytes().to_vec().into_boxed_slice())
            .decode()
            .expect("sealed bytes decode")
    }

    // (a) Ephemeral seal — signatures verify, id is 32 bytes, bytes non-empty; the seal key is a fresh
    //     ephemeral key (NOT the account key).
    #[test]
    fn ephemeral_seal_signatures_verify() {
        let partial = ephemeral_partial();
        let out = seal_and_encode_stealth_transfer(NETWORK, partial, &stealth_keys()).unwrap();
        assert!(!out.encoded_transaction.as_bytes().is_empty());
        assert_eq!(out.transaction_id.as_bytes().len(), 32);
        let tx = decode(&out);
        assert!(verify_all_signatures(&tx), "ephemeral seal signature must verify");
        let account_pk = public_key_bytes_from_secret(&fixed_scalar(11));
        assert_ne!(tx.seal_signature().public_key(), &account_pk, "not the account key");
    }

    // (b) Ephemeral seal — two independent runs both verify. The ephemeral seal key + nonce are random,
    //     so the keys differ across runs.
    #[test]
    fn ephemeral_seal_runs_are_independent_and_verify() {
        let a = seal_and_encode_stealth_transfer(NETWORK, ephemeral_partial(), &stealth_keys()).unwrap();
        let b = seal_and_encode_stealth_transfer(NETWORK, ephemeral_partial(), &stealth_keys()).unwrap();
        let (ta, tb) = (decode(&a), decode(&b));
        assert!(verify_all_signatures(&ta) && verify_all_signatures(&tb));
        assert_ne!(
            ta.seal_signature().public_key(),
            tb.seal_signature().public_key(),
            "the ephemeral seal key is freshly random per run"
        );
    }

    // (c) Stealth seal (c+k) — signatures verify; the seal public key equals (c+k)·G.
    #[test]
    fn stealth_seal_signatures_verify_and_key_matches() {
        let (partial, account_secret, public_nonce, _seed) = stealth_seal_partial();
        assert!(!partial.signature_requirements().must_sign_with_account_key);
        assert!(partial.signature_requirements().seal_signer.is_some());

        let out = seal_and_encode_stealth_transfer(NETWORK, partial, &stealth_keys()).unwrap();
        let tx = decode(&out);
        assert!(verify_all_signatures(&tx), "stealth seal signatures must verify");

        // The expected c+k seal key.
        let internal: InternalNetwork = NETWORK.into();
        let expected_secret = owner_stealth_dh_secret(
            internal,
            &account_secret,
            &fixed_scalar(VIEW_SECRET_SEED),
            &public_nonce,
        );
        let expected_pk = public_key_bytes_from_secret(&expected_secret);
        assert_eq!(
            tx.seal_signature().public_key(),
            &expected_pk,
            "seal signer public key must be (c+k)·G"
        );
    }

    // (d) Account-key seal — auth signature + seal signature both verify.
    #[test]
    fn account_key_seal_signatures_verify() {
        let partial = account_key_partial();
        assert!(partial.signature_requirements().must_sign_with_account_key);
        let out = seal_and_encode_stealth_transfer(NETWORK, partial, &stealth_keys()).unwrap();
        let tx = decode(&out);
        assert!(
            verify_all_signatures(&tx),
            "account-key auth + seal signatures must verify"
        );

        // The seal signer is the account key.
        let expected_pk = public_key_bytes_from_secret(&fixed_scalar(11));
        assert_eq!(tx.seal_signature().public_key(), &expected_pk);
    }

    // (e) All three production paths produce non-empty bytes + a 32-byte id.
    #[test]
    fn production_paths_produce_bytes_and_id() {
        // Ephemeral.
        let eph = seal_and_encode_stealth_transfer(NETWORK, ephemeral_partial(), &stealth_keys()).unwrap();
        assert!(!eph.encoded_transaction.as_bytes().is_empty());
        assert_eq!(eph.transaction_id.as_bytes().len(), 32);

        // Stealth seal.
        let (partial, _, _, _) = stealth_seal_partial();
        let st = seal_and_encode_stealth_transfer(NETWORK, partial, &stealth_keys()).unwrap();
        assert!(!st.encoded_transaction.as_bytes().is_empty());
        assert_eq!(st.transaction_id.as_bytes().len(), 32);
        assert!(verify_all_signatures(&decode(&st)));

        // Account key.
        let ak = seal_and_encode_stealth_transfer(NETWORK, account_key_partial(), &stealth_keys()).unwrap();
        assert!(!ak.encoded_transaction.as_bytes().is_empty());
        assert_eq!(ak.transaction_id.as_bytes().len(), 32);
        assert!(verify_all_signatures(&decode(&ak)));
    }

    // (f) main_signer. Ephemeral (0 auth sigs) ⇒ SealSigner. Account-key (1+ auth sigs) ⇒ also
    //     SealSigner (builder default; the auth sig covers the seal key per the stock semantics).
    #[test]
    fn seal_signer_is_the_main_signer() {
        let eph = seal_and_encode_stealth_transfer(NETWORK, ephemeral_partial(), &stealth_keys()).unwrap();
        let eph_tx = decode(&eph);
        assert!(
            eph_tx.is_seal_signer_authorized(),
            "ephemeral (0 auth sigs) ⇒ the seal signer must be the main signer"
        );

        let ak = seal_and_encode_stealth_transfer(NETWORK, account_key_partial(), &stealth_keys()).unwrap();
        assert!(
            decode(&ak).is_seal_signer_authorized(),
            "account-key path ⇒ the seal signer must be the main signer"
        );
    }

    // (g) The stealth-seal `c+k` key is deterministic across runs (it derives from the account secret +
    //     the input nonce), even though the seal *signature* is random. So the decoded seal public key +
    //     verification hold across two independent runs.
    #[test]
    fn stealth_seal_key_is_deterministic() {
        let (partial_a, _, _, _) = stealth_seal_partial();
        let (partial_b, _, _, _) = stealth_seal_partial();
        let a = seal_and_encode_stealth_transfer(NETWORK, partial_a, &stealth_keys()).unwrap();
        let b = seal_and_encode_stealth_transfer(NETWORK, partial_b, &stealth_keys()).unwrap();
        let (ta, tb) = (decode(&a), decode(&b));
        assert_eq!(
            ta.seal_signature().public_key(),
            tb.seal_signature().public_key(),
            "the c+k seal key is deterministic across runs"
        );
        assert!(verify_all_signatures(&ta) && verify_all_signatures(&tb));
    }

    // (h) Malformed account secret ⇒ KEY error, never a panic.
    #[test]
    fn malformed_account_secret_is_key_error() {
        let bad = StealthKeys::new(SecretKeyBytes::from_array([0xff; 32]), secret(VIEW_SECRET_SEED));
        let err = seal_and_encode_stealth_transfer(NETWORK, ephemeral_partial(), &bad).unwrap_err();
        assert_eq!(err.code(), "KEY");
    }

    // (i) Malformed view secret ⇒ KEY error, never a panic.
    #[test]
    fn malformed_view_secret_is_key_error() {
        let bad = StealthKeys::new(secret(11), SecretKeyBytes::from_array([0xff; 32]));
        let err = seal_and_encode_stealth_transfer(NETWORK, ephemeral_partial(), &bad).unwrap_err();
        assert_eq!(err.code(), "KEY");
    }
}
