//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Stealth **receive / scan** — the pure, RNG-free half of the confidential-transfer surface.
//!
//! [`scan_stealth_output`] tries to decrypt an inbound stealth UTXO with a view secret and decides
//! whether it belongs to the scanner. It assembles three already-written crypto leaves:
//!
//! 1. `encrypted_data_dh_kdf_aead(view_secret, sender_public_nonce)` derives the AEAD key — the inverse of the
//!    send-side `(nonce_secret, view_public_key)` derivation (DH commutativity).
//! 2. `unblind_output(commitment, encrypted_data, aead_key, skip_memo)` decrypts and re-checks the commitment,
//!    recovering the value + mask (+ optional memo).
//! 3. The UTXO scanning tag and the one-time spend public key `H(v·R)·G + A` are re-derived receiver-side from the view
//!    secret and the account **public** key and compared, confirming the output is spendable by that account.
//!
//! ## Return semantics
//!
//! - `Ok(Some(DecryptedOutput { is_mine: true, .. }))` — the AEAD decrypt succeeded, the tag (when present) matched
//!   and, for a `StealthPublicKey` output, the spend key matched.
//! - `Ok(None)` — **not ours**. An AEAD failure, a commitment mismatch, a tag mismatch, or a spend-key mismatch are all
//!   **normal** for UTXOs addressed to another recipient — never errors.
//! - `Err(OotleSdkError::Parse)` — structurally malformed inputs only (wrong byte widths, a non-canonical public key,
//!   an encrypted-data length out of the valid `[80, 335]` range).
//!
//! Every check needs only the view secret and the account public key, so a view-only wallet confirms
//! ownership as fully as a spending one.
//!
//! This module calls **no** RNG — decryption is the deterministic inverse of encryption, so the scan
//! output is byte-stable.

use ootle_network::Network as InternalNetwork;
use tari_crypto::{
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_ootle_wallet_crypto::{StealthCryptoApi, encrypted_data::unblind_output, kdfs, memo::Memo};
use tari_template_lib_types::{
    EncryptedData,
    ResourceAddress,
    crypto::{PedersenCommitmentBytes, UtxoTag},
};

use crate::types::{
    bytes::{PublicKeyBytes, SecretKeyBytes},
    error::OotleSdkError,
    network::Network,
    stealth::{CommitmentBytes, DecryptedOutput, EncryptedDataBytes, InboundStealthOutput, StealthMemo, StealthPayTo},
};

/// Parses a boundary view secret into the internal [`RistrettoSecretKey`].
fn parse_view_secret(secret: &SecretKeyBytes) -> Result<RistrettoSecretKey, OotleSdkError> {
    RistrettoSecretKey::from_canonical_bytes(secret.as_bytes())
        .map_err(|e| OotleSdkError::Parse(format!("invalid view secret: {e}")))
}

/// Parses a boundary account public key into the internal [`RistrettoPublicKey`].
fn parse_account_public_key(pk: &PublicKeyBytes) -> Result<RistrettoPublicKey, OotleSdkError> {
    RistrettoPublicKey::from_canonical_bytes(pk.as_bytes())
        .map_err(|e| OotleSdkError::Parse(format!("invalid account public key: {e}")))
}

/// Parses the sender's ephemeral public nonce into the internal [`RistrettoPublicKey`].
fn parse_sender_nonce(nonce: &PublicKeyBytes) -> Result<RistrettoPublicKey, OotleSdkError> {
    RistrettoPublicKey::from_canonical_bytes(nonce.as_bytes())
        .map_err(|e| OotleSdkError::Parse(format!("invalid sender_public_nonce: {e}")))
}

/// Parses a boundary spend public key into the internal [`RistrettoPublicKey`].
fn parse_spend_public_key(pk: &PublicKeyBytes) -> Result<RistrettoPublicKey, OotleSdkError> {
    RistrettoPublicKey::from_canonical_bytes(pk.as_bytes())
        .map_err(|e| OotleSdkError::Parse(format!("invalid spend_public_key: {e}")))
}

/// Parses the boundary commitment into the internal [`PedersenCommitmentBytes`].
fn parse_commitment(commitment: &CommitmentBytes) -> Result<PedersenCommitmentBytes, OotleSdkError> {
    PedersenCommitmentBytes::from_bytes(commitment.as_bytes())
        .map_err(|_| OotleSdkError::Parse("commitment: expected 32 bytes".to_string()))
}

/// Parses the boundary encrypted-data blob into the internal [`EncryptedData`], validating its
/// length is within the valid `[min_size(), max_size()]` range.
fn parse_encrypted_data(data: &EncryptedDataBytes) -> Result<EncryptedData, OotleSdkError> {
    EncryptedData::try_from(data.as_bytes().to_vec()).map_err(|len| {
        OotleSdkError::Parse(format!(
            "encrypted_data: invalid length {len}; expected [{}, {}]",
            EncryptedData::min_size(),
            EncryptedData::max_size()
        ))
    })
}

/// Maps a decoded internal [`Memo`] to the boundary [`StealthMemo`].
///
/// The boundary deliberately exposes only the two general-purpose variants; every other internal
/// variant (U256, pay-ref, sender-address) surfaces as [`StealthMemo::Bytes`] of its raw payload.
fn boundary_memo_from_internal(memo: &Memo) -> StealthMemo {
    match memo {
        Memo::Message(s) => StealthMemo::Message(s.to_string()),
        Memo::Bytes(b) => StealthMemo::Bytes(b.to_vec()),
        Memo::U256(b) => StealthMemo::Bytes(b.to_vec()),
        Memo::PayRefAndBytes(b) => StealthMemo::Bytes(b.to_vec()),
        Memo::SenderAddress(b) => StealthMemo::Bytes(b.to_vec()),
    }
}

/// Scans an inbound stealth UTXO: tries to decrypt it with `view_secret` and decides whether it is
/// addressed to the scanner.
///
/// This is the **pure, RNG-free** receive half of the confidential-transfer surface.
///
/// - `network` — the network whose hash domains parameterize the tag / spend-key derivation.
/// - `view_secret` — the recipient's view-only secret. Paired with `output.sender_public_nonce` to re-derive the AEAD
///   key (DH commutativity with the sender's `(nonce_secret, view_public_key)`).
/// - `account_public_key` — the recipient's **account** public key. With the view secret it re-derives the one-time
///   spend public key of a `StealthPublicKey` output, which must match the on-chain key.
/// - `output` — the inbound UTXO to scan.
/// - `skip_memo` — when `true`, the memo region is **not** decoded and the result's `memo` is `None` even if the
///   payload carries one (mirrors `unblind_output`'s `skip_memo`).
///
/// See the module docs for the full return semantics. In short: `Ok(Some(..))` = mine,
/// `Ok(None)` = not mine (a **normal** outcome), `Err(Parse)` = malformed input.
pub fn scan_stealth_output(
    network: Network,
    view_secret: &SecretKeyBytes,
    account_public_key: &PublicKeyBytes,
    output: &InboundStealthOutput,
    skip_memo: bool,
) -> Result<Option<DecryptedOutput>, OotleSdkError> {
    // Parse inputs (fail fast — only malformed bytes produce Err).
    let view_key = parse_view_secret(view_secret)?;
    let account_key = parse_account_public_key(account_public_key)?;
    let sender_nonce = parse_sender_nonce(&output.sender_public_nonce)?;
    let commitment = parse_commitment(&output.commitment)?;
    let enc_data = parse_encrypted_data(&output.encrypted_data)?;

    // Derive the AEAD encryption key: DH(view_secret, sender_public_nonce). The inverse of the
    // send-side `(nonce_secret, view_public_key)` derivation.
    let encryption_key = kdfs::encrypted_data_dh_kdf_aead(&view_key, &sender_nonce);

    // Decrypt. Any error (AEAD failure or commitment mismatch) => Ok(None) (not mine — normal for
    // UTXOs addressed to another recipient).
    let decrypted = match unblind_output(&commitment, &enc_data, &encryption_key, skip_memo) {
        Ok(d) => d,
        Err(_) => return Ok(None),
    };

    let internal_network: InternalNetwork = network.into();
    let crypto = StealthCryptoApi::new();

    // Tag check (only when the UTXO carries a tag). Receiver-side derivation mirrors the sender's
    // `derive_stealth_output_tag` but with `secret = view_secret`, `public_key = sender_public_nonce`
    // (the DH product is the same by commutativity).
    if let Some(tag_bytes) = &output.utxo_tag {
        let resource_address: ResourceAddress = output
            .resource_address
            .to_internal()
            .map_err(|e| OotleSdkError::Parse(format!("invalid resource address: {e}")))?;
        let expected_tag =
            crypto.derive_stealth_output_tag(internal_network, &view_key, &sender_nonce, &resource_address);
        if expected_tag != UtxoTag::new(tag_bytes.to_u32()) {
            return Ok(None); // tag mismatch — not mine
        }
    }

    // Authorisation check for a `StealthPublicKey` output carrying a one-time spend key `H(v·R)·G + A`.
    // A sender can encrypt to our view key while committing a spend key it controls, so a successful
    // decrypt alone does not make the output ours. A `StealthPublicKey` output ALWAYS carries the
    // one-time spend key on the wire; its absence is malformed input.
    if output.pay_to == StealthPayTo::StealthPublicKey {
        let spend_pk_bytes = output.spend_public_key.as_ref().ok_or_else(|| {
            OotleSdkError::Parse("StealthPublicKey output is missing its spend_public_key".to_string())
        })?;
        let derived_spend_pk = crypto.derive_stealth_owner_public_key_from_view_key(
            internal_network,
            &account_key,
            &view_key,
            &sender_nonce,
        );
        let expected_spend_pk = parse_spend_public_key(spend_pk_bytes)?;
        if derived_spend_pk != expected_spend_pk {
            return Ok(None); // spend-key mismatch — not addressed to us
        }
    }

    // Build the decrypted output. The mask is a `RistrettoSecretKey`, always exactly 32 bytes, so
    // the width conversion is infallible.
    let memo = decrypted.memo().map(boundary_memo_from_internal);
    let mask = SecretKeyBytes::from_bytes(decrypted.mask().as_bytes())
        .expect("RistrettoSecretKey is always 32 bytes — width is guaranteed");

    Ok(Some(DecryptedOutput {
        value: decrypted.value(),
        mask,
        memo,
        is_mine: true,
    }))
}

/// Fused **decode → scan**: turn a fetched UTXO substate into an [`InboundStealthOutput`] and scan it
/// in one call.
///
/// This is `decode_stealth_utxo` then [`scan_stealth_output`] — the convenience the receive path uses
/// so a host fetches a substate and learns whether it is theirs without first marshalling the decoded
/// shape back out and in. The decode is the **single shared** one in
/// [`crate::stealth::decode`]; the scan is the same RNG-free crypto.
///
/// - `substate_id` — the UTXO's canonical address (`utxo_<resource>_<commitment>`), carrying the commitment + resource
///   address the value body omits.
/// - `substate_value` — the [`SubstateValue`] the indexer returned, verbatim.
///
/// Returns `Ok(None)` for a "not mine" output exactly like [`scan_stealth_output`] (a decrypt-miss is
/// normal, never an error). A malformed substate id ⇒ `Parse`; a non-UTXO / frozen / burnt substate
/// or a malformed nonce ⇒ `Invalid` / `Key` (from the decode).
pub fn scan_stealth_substate(
    network: Network,
    view_secret: &SecretKeyBytes,
    account_public_key: &PublicKeyBytes,
    substate_id: &str,
    substate_value: &serde_json::Value,
    skip_memo: bool,
) -> Result<Option<DecryptedOutput>, OotleSdkError> {
    let inbound = crate::stealth::decode::decode_stealth_utxo(substate_id, substate_value)?;
    scan_stealth_output(network, view_secret, account_public_key, &inbound, skip_memo)
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{
        commitment::HomomorphicCommitmentFactory,
        keys::PublicKey as _,
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
        tari_utilities::ByteArray,
    };
    use tari_engine_types::crypto::get_commitment_factory;
    use tari_ootle_wallet_crypto::{encrypted_data::encrypt_data, kdfs, memo::Memo};

    use super::*;
    use crate::types::{address::ResourceAddressStr, bytes::PublicKeyBytes, stealth::UtxoTagBytes};

    fn scalar(seed: u8) -> RistrettoSecretKey {
        let mut b = [0u8; 32];
        b[0] = seed;
        RistrettoSecretKey::from_canonical_bytes(&b).expect("canonical low scalar")
    }

    fn secret_bytes(sk: &RistrettoSecretKey) -> SecretKeyBytes {
        SecretKeyBytes::from_bytes(sk.as_bytes()).expect("32-byte secret")
    }

    fn pk_bytes(pk: &RistrettoPublicKey) -> PublicKeyBytes {
        PublicKeyBytes::from_bytes(pk.as_bytes()).expect("32-byte pk")
    }

    fn tari_resource() -> ResourceAddressStr {
        ResourceAddressStr::parse(tari_template_lib_types::constants::STEALTH_TARI_RESOURCE_ADDRESS.to_string())
            .expect("valid resource")
    }

    /// Builds an inbound stealth UTXO addressed to (`view_sk`, `account_pk`) for `amount`, with an
    /// optional memo and (when `with_tag`/`with_spend_key`) the ownership material a real send would
    /// emit. Returns the UTXO plus its commitment mask (for assertions).
    #[allow(clippy::too_many_arguments)]
    fn make_inbound(
        network: Network,
        account_pk: &RistrettoPublicKey,
        view_pk: &RistrettoPublicKey,
        nonce_secret: &RistrettoSecretKey,
        mask: &RistrettoSecretKey,
        amount: u64,
        memo: Option<&Memo>,
        with_tag: bool,
        with_spend_key: bool,
    ) -> InboundStealthOutput {
        let internal_network: InternalNetwork = network.into();
        let public_nonce = RistrettoPublicKey::from_secret_key(nonce_secret);
        let crypto = StealthCryptoApi::new();

        // Sender-side AEAD key: (nonce_secret, view_public_key).
        let encryption_key = kdfs::encrypted_data_dh_kdf_aead(nonce_secret, view_pk);
        let encrypted_data = encrypt_data(amount, mask, &encryption_key, memo).expect("encrypt");
        let commitment = get_commitment_factory().commit_value(mask, amount).to_byte_type();

        let resource = tari_resource();
        let resource_internal = resource.to_internal().unwrap();

        let (pay_to, spend_public_key) = if with_spend_key {
            let owner_pk = crypto.derive_stealth_owner_public_key(internal_network, account_pk, view_pk, nonce_secret);
            (
                StealthPayTo::StealthPublicKey,
                Some(pk_bytes(
                    &RistrettoPublicKey::from_canonical_bytes(owner_pk.as_bytes()).unwrap(),
                )),
            )
        } else {
            (StealthPayTo::AccessRuleAllowAll, None)
        };

        let utxo_tag = with_tag.then(|| {
            // Sender-side tag: derive_stealth_output_tag(network, nonce_secret, view_pk, resource).
            let tag = crypto.derive_stealth_output_tag(internal_network, nonce_secret, view_pk, &resource_internal);
            UtxoTagBytes::from_u32(tag.value())
        });

        InboundStealthOutput {
            commitment: CommitmentBytes::from_bytes(commitment.as_bytes()).unwrap(),
            encrypted_data: EncryptedDataBytes::from_bytes(encrypted_data.as_bytes()),
            sender_public_nonce: pk_bytes(&public_nonce),
            pay_to,
            spend_public_key,
            utxo_tag,
            resource_address: resource,
        }
    }

    // (a) Round-trip: encrypt → scan recovers value + mask.
    #[test]
    fn round_trip_recovers_value_and_mask() {
        let net = Network::LocalNet;
        let view_sk = scalar(20);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_sk = scalar(21);
        let account_pk = RistrettoPublicKey::from_secret_key(&account_sk);
        let nonce_secret = scalar(22);
        let mask = scalar(23);
        let amount = 12_345u64;

        let utxo = make_inbound(
            net,
            &account_pk,
            &view_pk,
            &nonce_secret,
            &mask,
            amount,
            None,
            true,
            true,
        );

        let got = scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, false)
            .unwrap()
            .expect("should be mine");
        assert!(got.is_mine);
        assert_eq!(got.value, amount);
        assert_eq!(got.mask, secret_bytes(&mask));
        assert!(got.memo.is_none());
    }

    // (b) Not mine → Ok(None): a different view secret yields the wrong AEAD key.
    #[test]
    fn wrong_view_key_is_not_mine() {
        let net = Network::LocalNet;
        let view_sk = scalar(30);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_pk = RistrettoPublicKey::from_secret_key(&scalar(31));
        let nonce_secret = scalar(32);
        let mask = scalar(33);

        let utxo = make_inbound(
            net,
            &account_pk,
            &view_pk,
            &nonce_secret,
            &mask,
            999,
            None,
            false,
            false,
        );

        // Scan with a different view secret.
        let other_view = scalar(99);
        let got = scan_stealth_output(net, &secret_bytes(&other_view), &pk_bytes(&account_pk), &utxo, false).unwrap();
        assert!(got.is_none(), "wrong view key must be Ok(None)");
    }

    // (c) The view key and account public key alone confirm a tagged, key-path output.
    #[test]
    fn view_key_confirms_ownership() {
        let net = Network::LocalNet;
        let view_sk = scalar(40);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_pk = RistrettoPublicKey::from_secret_key(&scalar(41));
        let nonce_secret = scalar(42);
        let mask = scalar(43);
        let amount = 7_000u64;

        let utxo = make_inbound(
            net,
            &account_pk,
            &view_pk,
            &nonce_secret,
            &mask,
            amount,
            None,
            true,
            true,
        );

        let got = scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, false)
            .unwrap()
            .expect("view key confirms ownership");
        assert_eq!(got.value, amount);
        assert_eq!(got.mask, secret_bytes(&mask));
    }

    // (d) Memo recovery (skip_memo = false).
    #[test]
    fn memo_is_recovered() {
        let net = Network::LocalNet;
        let view_sk = scalar(50);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_pk = RistrettoPublicKey::from_secret_key(&scalar(51));
        let nonce_secret = scalar(52);
        let mask = scalar(53);
        let memo = Memo::new_message("gm ser").unwrap();

        let utxo = make_inbound(
            net,
            &account_pk,
            &view_pk,
            &nonce_secret,
            &mask,
            1,
            Some(&memo),
            false,
            false,
        );

        let got = scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, false)
            .unwrap()
            .expect("mine");
        assert_eq!(got.memo, Some(StealthMemo::Message("gm ser".to_string())));
    }

    // (e) skip_memo = true → memo.is_none() even when a memo is present.
    #[test]
    fn skip_memo_drops_present_memo() {
        let net = Network::LocalNet;
        let view_sk = scalar(60);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_pk = RistrettoPublicKey::from_secret_key(&scalar(61));
        let nonce_secret = scalar(62);
        let mask = scalar(63);
        let memo = Memo::new_message("secret").unwrap();

        let utxo = make_inbound(
            net,
            &account_pk,
            &view_pk,
            &nonce_secret,
            &mask,
            1,
            Some(&memo),
            false,
            false,
        );

        let got = scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, true)
            .unwrap()
            .expect("mine");
        assert!(got.memo.is_none(), "skip_memo must drop the memo");
    }

    // (f) Malformed public material (non-canonical sender nonce) → Err with code() == "PARSE".
    //
    // The `CommitmentBytes`/`PublicKeyBytes` newtypes are fixed-width (a 31-byte commitment cannot be
    // constructed), so the "structurally malformed" case the public fn surfaces is a non-canonical
    // point. The parse helper's wrong-width contract is exercised separately below.
    #[test]
    fn malformed_public_material_is_parse_error() {
        let net = Network::LocalNet;
        let view_sk = scalar(70);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_pk = RistrettoPublicKey::from_secret_key(&scalar(71));
        let nonce_secret = scalar(72);
        let mask = scalar(73);

        // The commitment parse helper rejects a wrong-width slice.
        assert!(super::parse_commitment(&CommitmentBytes::from_array([0u8; 32])).is_ok());
        assert!(PedersenCommitmentBytes::from_bytes(&[0u8; 31]).is_err());

        // A non-canonical sender nonce (all-0xff is not a canonical Ristretto point) → PARSE.
        let mut utxo = make_inbound(net, &account_pk, &view_pk, &nonce_secret, &mask, 5, None, false, false);
        utxo.sender_public_nonce = PublicKeyBytes::from_array([0xff; 32]);
        let scan_err =
            scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, false).unwrap_err();
        assert_eq!(scan_err.code(), "PARSE");
    }

    // A non-canonical account public key → Err with code() == "PARSE".
    #[test]
    fn malformed_account_public_key_is_parse_error() {
        let net = Network::LocalNet;
        let view_sk = scalar(75);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_pk = RistrettoPublicKey::from_secret_key(&scalar(76));

        let utxo = make_inbound(
            net,
            &account_pk,
            &view_pk,
            &scalar(77),
            &scalar(78),
            5,
            None,
            true,
            true,
        );
        let err = scan_stealth_output(
            net,
            &secret_bytes(&view_sk),
            &PublicKeyBytes::from_array([0xff; 32]),
            &utxo,
            false,
        )
        .unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    // (g) Encrypted data too short (< min_size) → Err with code() == "PARSE".
    #[test]
    fn short_encrypted_data_is_parse_error() {
        let net = Network::LocalNet;
        let view_sk = scalar(80);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_pk = RistrettoPublicKey::from_secret_key(&scalar(81));
        let nonce_secret = scalar(82);
        let mask = scalar(83);

        let mut utxo = make_inbound(net, &account_pk, &view_pk, &nonce_secret, &mask, 5, None, false, false);
        utxo.encrypted_data = EncryptedDataBytes::from_bytes(&[0u8; 10]); // < 80
        let err = scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, false).unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    // (h) Tag mismatch → Ok(None) (not mine): AEAD decrypts but the tag does not match.
    #[test]
    fn tag_mismatch_is_not_mine() {
        let net = Network::LocalNet;
        let view_sk = scalar(90);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_sk = scalar(91);
        let account_pk = RistrettoPublicKey::from_secret_key(&account_sk);
        let nonce_secret = scalar(92);
        let mask = scalar(93);

        let mut utxo = make_inbound(net, &account_pk, &view_pk, &nonce_secret, &mask, 5, None, true, true);
        // Corrupt the tag.
        utxo.utxo_tag = Some(UtxoTagBytes::from_u32(0xDEAD_BEEF));
        let got = scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, false).unwrap();
        assert!(got.is_none(), "tag mismatch must be Ok(None)");
    }

    // Spend-key mismatch → Ok(None): the output decrypts and its tag matches, but the on-chain spend key
    // is one the sender chose rather than ours.
    #[test]
    fn spend_key_mismatch_is_not_mine() {
        let net = Network::LocalNet;
        let view_sk = scalar(100);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_sk = scalar(101);
        let account_pk = RistrettoPublicKey::from_secret_key(&account_sk);
        let nonce_secret = scalar(102);
        let mask = scalar(103);

        let mut utxo = make_inbound(net, &account_pk, &view_pk, &nonce_secret, &mask, 5, None, true, true);
        // Replace the spend key with one for a different account.
        let other = RistrettoPublicKey::from_secret_key(&scalar(199));
        utxo.spend_public_key = Some(pk_bytes(&other));
        let got = scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, false).unwrap();
        assert!(got.is_none(), "spend-key mismatch must be Ok(None)");
    }

    // A StealthPublicKey output missing its on-chain spend key is structurally malformed → Err with code() == "PARSE",
    // not a false-positive is_mine.
    #[test]
    fn stealth_public_key_without_spend_key_is_parse_error() {
        let net = Network::LocalNet;
        let view_sk = scalar(120);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_sk = scalar(121);
        let account_pk = RistrettoPublicKey::from_secret_key(&account_sk);
        let nonce_secret = scalar(122);
        let mask = scalar(123);

        let mut utxo = make_inbound(net, &account_pk, &view_pk, &nonce_secret, &mask, 5, None, false, true);
        utxo.spend_public_key = None; // StealthPublicKey but no key on the wire — malformed.
        let err = scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, false).unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    // (i) Reproducibility: two calls with identical inputs return identical output (pure).
    #[test]
    fn scan_is_reproducible() {
        let net = Network::LocalNet;
        let view_sk = scalar(110);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_sk);
        let account_sk = scalar(111);
        let account_pk = RistrettoPublicKey::from_secret_key(&account_sk);
        let nonce_secret = scalar(112);
        let mask = scalar(113);

        let utxo = make_inbound(net, &account_pk, &view_pk, &nonce_secret, &mask, 42, None, true, true);
        let a = scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, false).unwrap();
        let b = scan_stealth_output(net, &secret_bytes(&view_sk), &pk_bytes(&account_pk), &utxo, false).unwrap();
        assert_eq!(a, b);
    }
}
