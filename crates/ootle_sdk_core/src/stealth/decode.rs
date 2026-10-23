//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Stealth **UTXO decode** — turn a fetched UTXO substate (what the indexer returns) into the
//! receive-shaped [`InboundStealthOutput`] the scanner consumes, **and** the single shared field
//! extraction the spend path's input resolver reuses.
//!
//! ## Shared field extraction
//!
//! Both the receive path ([`decode_stealth_utxo`]) and the spend path
//! ([`resolve_one_stealth_utxo`](crate::stealth::inputs::resolve_one_stealth_utxo)) need the same
//! per-UTXO validation + field extraction: a [`SubstateValue`] must be a [`Utxo`] that is **not**
//! frozen and **not** burnt, and its [`OutputBody::public_nonce`] must recover a canonical
//! [`RistrettoPublicKey`]. That extraction lives **once** here, in [`extract_utxo_output`], and is
//! called by both paths so the two can never drift.
//!
//! [`decode_stealth_utxo`] then maps the extracted [`UtxoOutput`] plus the UTXO's **substate id**
//! (which carries the on-chain commitment and resource address — the value body does **not**) onto an
//! [`InboundStealthOutput`]. The spend path keeps decrypting directly off the extracted output, so it
//! does not build an `InboundStealthOutput` itself.
//!
//! ## Why the substate id is required
//!
//! A [`SubstateValue::Utxo`] carries only the [`UtxoOutput`] (public nonce, encrypted data, spend
//! condition, tag) — **not** the commitment or the resource address. Those live in the substate
//! **address** (`utxo_<resource>_<commitment>`), which the host already holds for every fetched
//! substate. So [`decode_stealth_utxo`] takes both the id and the value (the host passes exactly what
//! the indexer returned, verbatim — id + value JSON), and is the only way to produce a *complete*
//! `InboundStealthOutput`.
//!
//! ## Purity
//!
//! This module calls **no** RNG — it is a pure parse + field map. The downstream scan is likewise
//! RNG-free, so the fused [`scan_stealth_substate`](crate::scan_stealth_substate) is byte-stable.

use std::str::FromStr;

use ootle_byte_type::FromByteType;
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tari_engine_types::{
    UtxoOutput,
    substate::{SubstateId, SubstateValue},
};
use tari_ootle_wallet_crypto::stealth::condition_root;
use tari_template_lib_types::{
    access_rules::AccessRule,
    stealth::{SpendAuthorization, SpendCondition},
};

use crate::types::{
    address::ResourceAddressStr,
    bytes::PublicKeyBytes,
    error::OotleSdkError,
    stealth::{CommitmentBytes, EncryptedDataBytes, InboundStealthOutput, StealthPayTo, UtxoTagBytes},
};

/// Validates a fetched UTXO substate and extracts the recovered sender public nonce + output body.
///
/// This is the **single shared decode** between the receive path ([`decode_stealth_utxo`]) and the
/// spend path's input resolver. It checks:
///
/// - the value must be a [`Utxo`](tari_engine_types::Utxo) substate (else `Invalid`),
/// - the UTXO must not be frozen (else `Invalid`),
/// - the UTXO must not be burnt — it must carry an output body (else `Invalid`),
/// - the body's [`public_nonce`](tari_engine_types::crypto::OutputBody::public_nonce) must recover a canonical
///   [`RistrettoPublicKey`] (else `Key`).
///
/// `context` is a caller-supplied label (the substate id for the spend path, a generic description
/// for the receive path) woven into the error messages so failures stay diagnosable from either site.
///
/// Returns the cloned [`UtxoOutput`] (the spend path needs `encrypted_data` + `auth`; the
/// receive path needs all of it) plus the recovered nonce.
pub(crate) fn extract_utxo_output(
    value: &SubstateValue,
    context: &str,
) -> Result<(RistrettoPublicKey, UtxoOutput), OotleSdkError> {
    // It must be a UTXO substate.
    let utxo = value
        .as_utxo()
        .ok_or_else(|| OotleSdkError::Invalid(format!("expected a UTXO substate at '{context}'")))?;

    // Reject a frozen UTXO.
    if utxo.is_frozen {
        return Err(OotleSdkError::Invalid(format!("stealth UTXO '{context}' is frozen")));
    }

    // Reject a burnt UTXO (no output body).
    let output = utxo
        .output
        .clone()
        .ok_or_else(|| OotleSdkError::Invalid(format!("stealth UTXO '{context}' is burnt")))?;

    // Recover the sender public nonce from the output body.
    let public_nonce: RistrettoPublicKey = output
        .output
        .public_nonce
        .try_from_byte_type()
        .map_err(|e| OotleSdkError::Key(format!("malformed public nonce in stealth UTXO '{context}': {e}")))?;

    Ok((public_nonce, output))
}

/// Decodes a fetched UTXO substate into the receive-shaped [`InboundStealthOutput`] the scanner
/// consumes.
///
/// `substate_id` is the UTXO's canonical address string (`utxo_<resource>_<commitment>`) — it carries
/// the on-chain commitment and resource address, which the value body does **not** (see the module
/// docs). `substate_value` is the [`SubstateValue`] JSON the indexer returned, passed verbatim.
///
/// Field map:
/// - `commitment` / `resource_address` ← parsed from `substate_id`,
/// - `encrypted_data` / `sender_public_nonce` ← the output body,
/// - `pay_to` / `spend_public_key` ← the UTXO's [`SpendAuthorization`] ([`Key`](SpendAuthorization::Key) ⇒
///   [`StealthPublicKey`](StealthPayTo::StealthPublicKey) + the one-time spend key; the single-leaf
///   `AccessRule::AllowAll` [`Script`](SpendAuthorization::Script) root ⇒
///   [`AccessRuleAllowAll`](StealthPayTo::AccessRuleAllowAll)),
/// - `utxo_tag` ← the UTXO's tag.
///
/// `substate_value` is the indexer's `SubstateValue` JSON, passed through verbatim — the same neutral
/// carrier [`FetchedSubstate`](crate::FetchedSubstate) uses, so the facade stays a thin JSON marshal.
///
/// Errors: a non-UTXO / frozen / burnt substate or a malformed nonce surface as `Invalid` / `Key`
/// (via [`extract_utxo_output`]); a malformed `substate_id` or undecodable value JSON surface as
/// `Parse` / `Encoding`.
pub fn decode_stealth_utxo(
    substate_id: &str,
    substate_value: &serde_json::Value,
) -> Result<InboundStealthOutput, OotleSdkError> {
    let id = SubstateId::from_str(substate_id)
        .map_err(|e| OotleSdkError::Parse(format!("invalid substate id '{substate_id}': {e}")))?;
    let utxo_address = id
        .as_utxo_address()
        .ok_or_else(|| OotleSdkError::Parse(format!("substate id '{substate_id}' is not a UTXO address")))?;

    // The commitment + resource address live in the address, not the value body.
    let resource_address = ResourceAddressStr::parse(utxo_address.resource_address().to_string())
        .map_err(|e| OotleSdkError::Parse(format!("invalid resource address in '{substate_id}': {e}")))?;
    let commitment = CommitmentBytes::from_bytes(utxo_address.id().as_bytes())
        .map_err(|e| OotleSdkError::Parse(format!("invalid commitment in '{substate_id}': {e}")))?;

    // Parse the indexer's JSON into the internal SubstateValue (undecodable bytes ⇒ Encoding,
    // matching the spend site's convention).
    let value: SubstateValue = serde_json::from_value(substate_value.clone())
        .map_err(|e| OotleSdkError::Encoding(format!("undecodable substate value for '{substate_id}': {e}")))?;

    // Validate the substate + recover the nonce via the shared extraction.
    let (public_nonce, output) = extract_utxo_output(&value, substate_id)?;

    let sender_public_nonce = PublicKeyBytes::from_bytes(public_nonce.as_bytes())
        .map_err(|e| OotleSdkError::Key(format!("public nonce is not 32 bytes: {e}")))?;
    let encrypted_data = EncryptedDataBytes::from_bytes(output.output.encrypted_data.as_bytes());

    // Map the on-chain spend authorisation onto the boundary pay-to selector + the one-time spend key. A
    // condition tree commits only an opaque root, so the only script-path output the SDK can recognise is the
    // single-leaf `AccessRule::AllowAll` tree it builds itself; any other committed root (an arbitrary TIP-0006
    // spend script) is not introspectable here and is unsupported.
    let allow_all_root = condition_root(&[SpendCondition::access_rule(AccessRule::AllowAll)])
        .map_err(|e| OotleSdkError::Stealth(format!("compute allow-all condition root: {e}")))?;
    let (pay_to, spend_public_key) = match &output.auth {
        SpendAuthorization::Key(pk) => {
            let spend_pk = PublicKeyBytes::from_bytes(pk.as_bytes())
                .map_err(|e| OotleSdkError::Key(format!("malformed spend public key in '{substate_id}': {e}")))?;
            (StealthPayTo::StealthPublicKey, Some(spend_pk))
        },
        SpendAuthorization::Script(root) if *root == allow_all_root => (StealthPayTo::AccessRuleAllowAll, None),
        SpendAuthorization::Script(_) | SpendAuthorization::KeyAndScript { .. } => {
            return Err(OotleSdkError::Stealth(format!(
                "spend-script (TIP-0006) UTXO '{substate_id}' is not supported by the SDK scan/decode path"
            )));
        },
    };

    // The scanning tag is only meaningful for a stealth-addressed (`Signed`) output: the receiver
    // re-derives and matches it against their view secret. For an `AccessRuleAllowAll` output there
    // is no stealth addressing, so we surface `None` (mirroring `spend_public_key = None` above) —
    // otherwise the scan's opt-in tag check would run on a semantically-undefined on-chain tag and
    // could yield a spurious not-mine.
    let utxo_tag = match pay_to {
        StealthPayTo::StealthPublicKey => Some(UtxoTagBytes::from_u32(output.tag.value())),
        StealthPayTo::AccessRuleAllowAll => None,
    };

    Ok(InboundStealthOutput {
        commitment,
        encrypted_data,
        sender_public_nonce,
        pay_to,
        spend_public_key,
        utxo_tag,
        resource_address,
    })
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{
        keys::{PublicKey as _, SecretKey as _},
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    };
    use tari_engine_types::{
        Utxo,
        UtxoOutput,
        crypto::{OutputBody, commit_u64_amount},
        substate::SubstateValue,
    };
    use tari_ootle_wallet_crypto::{StealthCryptoApi, encrypted_data::encrypt_data, kdfs};
    use tari_template_lib_types::{ObjectKey, ResourceAddress, access_rules::AccessRule, stealth::SpendCondition};

    use super::*;
    use crate::{
        scan_stealth_output,
        stealth::stealth_utxo_substate_id,
        types::{bytes::SecretKeyBytes, network::Network},
    };

    fn secret(seed: u8) -> RistrettoSecretKey {
        RistrettoSecretKey::from_uniform_bytes(&[seed; 64]).unwrap()
    }

    fn resource() -> ResourceAddress {
        ResourceAddress::new(ObjectKey::from_array([0xcc; ObjectKey::LENGTH]))
    }

    /// A fabricated stealth UTXO addressed to (`view_secret`, `account_pk`) for `(value, mask)`, built
    /// from the real send-side crypto. Returns the substate id, the substate value, and the scanning
    /// keys for a decode → scan round-trip.
    struct Built {
        substate_id: String,
        value: serde_json::Value,
        view_secret: RistrettoSecretKey,
        account_secret: RistrettoSecretKey,
        mask: RistrettoSecretKey,
        amount: u64,
    }

    fn build(network: Network, value: u64, with_signed: bool) -> Built {
        let internal_network: ootle_network::Network = network.into();
        let mask = secret(11);
        let nonce_secret = secret(12);
        let public_nonce = RistrettoPublicKey::from_secret_key(&nonce_secret);
        let view_secret = secret(13);
        let view_pk = RistrettoPublicKey::from_secret_key(&view_secret);
        let account_secret = secret(14);
        let account_pk = RistrettoPublicKey::from_secret_key(&account_secret);
        let crypto = StealthCryptoApi::new();

        let encryption_key = kdfs::encrypted_data_dh_kdf_aead(&view_secret, &public_nonce);
        let encrypted_data = encrypt_data(value, &mask, &encryption_key, None).unwrap();
        let commitment = commit_u64_amount(&mask, value).to_byte_type();
        let commitment_hex = hex::encode(commitment.as_bytes());

        let resource_internal = resource();
        let tag = crypto.derive_stealth_output_tag(internal_network, &nonce_secret, &view_pk, &resource_internal);

        let auth = if with_signed {
            let owner_pk =
                crypto.derive_stealth_owner_public_key(internal_network, &account_pk, &view_pk, &nonce_secret);
            SpendAuthorization::Key(owner_pk.to_byte_type())
        } else {
            SpendAuthorization::Script(condition_root(&[SpendCondition::access_rule(AccessRule::AllowAll)]).unwrap())
        };

        let output_body = OutputBody {
            public_nonce: public_nonce.to_byte_type(),
            encrypted_data,
            minimum_value_promise: 0,
            viewable_balance: None,
        };
        let utxo = Utxo::new(UtxoOutput {
            output: output_body,
            auth,
            tag,
        });

        let substate_id = stealth_utxo_substate_id(&resource().to_string(), &commitment_hex)
            .unwrap()
            .to_string();

        Built {
            substate_id,
            value: serde_json::to_value(SubstateValue::Utxo(utxo)).expect("SubstateValue serializes"),
            view_secret,
            account_secret,
            mask,
            amount: value,
        }
    }

    // (a) decode → scan round-trips a real fabricated UTXO: the decoded InboundStealthOutput scans as
    //     mine with the recovered value + mask.
    #[test]
    fn decode_then_scan_round_trips() {
        let net = Network::LocalNet;
        let b = build(net, 7_654u64, true);

        let inbound = decode_stealth_utxo(&b.substate_id, &b.value).expect("decode");
        assert_eq!(inbound.pay_to, StealthPayTo::StealthPublicKey);
        assert!(inbound.spend_public_key.is_some());
        assert!(inbound.utxo_tag.is_some());

        let view = SecretKeyBytes::from_bytes(b.view_secret.as_bytes()).unwrap();
        let account =
            PublicKeyBytes::from_bytes(RistrettoPublicKey::from_secret_key(&b.account_secret).as_bytes()).unwrap();
        let got = scan_stealth_output(net, &view, &account, &inbound, true)
            .unwrap()
            .expect("should be mine");
        assert!(got.is_mine);
        assert_eq!(got.value, b.amount);
        assert_eq!(got.mask, SecretKeyBytes::from_bytes(b.mask.as_bytes()).unwrap());
    }

    // (b) an AccessRule-spend UTXO decodes to AccessRuleAllowAll with no spend key.
    #[test]
    fn decode_access_rule_has_no_spend_key() {
        let net = Network::LocalNet;
        let b = build(net, 1u64, false);
        let inbound = decode_stealth_utxo(&b.substate_id, &b.value).expect("decode");
        assert_eq!(inbound.pay_to, StealthPayTo::AccessRuleAllowAll);
        assert!(inbound.spend_public_key.is_none());
        // An AccessRule output has no stealth-addressing tag (mirrors spend_public_key = None).
        assert!(inbound.utxo_tag.is_none());
    }

    // (c) a burnt UTXO (output: None) ⇒ Invalid (via the shared extraction).
    #[test]
    fn burnt_substate_is_invalid() {
        let net = Network::LocalNet;
        let b = build(net, 1u64, true);
        // Burn the UTXO by nulling its output body in the JSON.
        let burnt = serde_json::json!({ "Utxo": { "is_frozen": false, "output": null } });
        let err = decode_stealth_utxo(&b.substate_id, &burnt).unwrap_err();
        assert_eq!(err.code(), "INVALID");
    }

    // (c2) an undecodable substate value JSON ⇒ Encoding.
    #[test]
    fn undecodable_substate_value_is_encoding_error() {
        let net = Network::LocalNet;
        let b = build(net, 1u64, true);
        let bad = serde_json::json!({ "NotASubstate": 7 });
        let err = decode_stealth_utxo(&b.substate_id, &bad).unwrap_err();
        assert_eq!(err.code(), "ENCODING");
    }

    // (d) a malformed substate id ⇒ Parse.
    #[test]
    fn malformed_substate_id_is_parse_error() {
        let net = Network::LocalNet;
        let b = build(net, 1u64, true);
        let err = decode_stealth_utxo("not-a-substate-id", &b.value).unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }

    // (e) a non-UTXO substate id (component address) ⇒ Parse.
    #[test]
    fn non_utxo_substate_id_is_parse_error() {
        let net = Network::LocalNet;
        let b = build(net, 1u64, true);
        let component_id = "component_a8d8d883f5f50e5a8b4c8a8a8b8c8d8e8f808182838485868788898a8b8c8d8e";
        let err = decode_stealth_utxo(component_id, &b.value).unwrap_err();
        assert_eq!(err.code(), "PARSE");
    }
}
