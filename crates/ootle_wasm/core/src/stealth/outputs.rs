//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Building the output side of a stealth transfer: per-output commitments + encrypted data, optional
//! ElGamal view-key proofs, and the aggregated bulletproof range proof.

use std::str::FromStr;

use ootle_byte_type::ToByteType;
use ootle_network::Network;
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_ootle_wallet_crypto::{
    OutputWitness,
    StealthCryptoApi,
    StealthOutputWitness,
    bullet_proof::generate_extended_bullet_proof as crypto_generate_extended_bullet_proof,
    memo::Memo,
    pay_to::PayTo,
    stealth::{create_outputs_statement, pay_to_output_authorization},
};
use tari_template_lib_types::{
    Amount,
    ResourceAddress,
    crypto::{RangeProofBytes, RistrettoPublicKeyBytes},
    stealth::RevealedOutput,
};

use crate::{
    error::OotleWasmError,
    keys::public_key_from_bytes,
    stealth::types::{OutputWitnessJson, StealthOutputWitnessJson},
};

/// Result of [`generate_stealth_outputs_statement`]: the wire-ready statement plus the aggregated mask the
/// sender retains for the balance proof.
#[derive(Debug, Clone)]
pub struct StealthOutputsResult {
    /// Serialized `StealthOutputsStatement` (containing outputs, revealed amount, and aggregated range
    /// proof).
    pub statement_json: String,
    /// Sum of all witness masks as a 32-byte Ristretto scalar. Feed this into
    /// [`crate::stealth::balance_proof::generate_stealth_balance_proof_signature`] as the aggregated
    /// output mask.
    pub aggregated_output_mask: Vec<u8>,
}

/// Generate the output half of a stealth transfer.
///
/// `witnesses_json` is a JSON array of [`StealthOutputWitnessJson`]; `revealed_output_amount_microtari` is
/// the plaintext revealed amount (in microtari, fits a `u64`). Output witnesses that contain a
/// `resource_view_key` automatically receive an ElGamal viewable-balance proof.
pub fn generate_stealth_outputs_statement(
    witnesses_json: &str,
    revealed_output_amount_microtari: u64,
    revealed_receiver: &[u8],
) -> Result<StealthOutputsResult, OotleWasmError> {
    use tari_ootle_wallet_crypto::StealthOutputWitness;

    let witnesses: Vec<StealthOutputWitnessJson> = serde_json::from_str(witnesses_json)?;
    let witnesses: Vec<StealthOutputWitness> = witnesses
        .into_iter()
        .map(StealthOutputWitness::try_from)
        .collect::<Result<Vec<_>, OotleWasmError>>()?;

    let aggregated_output_mask = witnesses
        .iter()
        .map(|w| &w.witness.mask)
        .fold(RistrettoSecretKey::default(), |acc, mask| acc + mask);

    let revealed_output = match revealed_output_amount_microtari {
        0 => None,
        amount => {
            let receiver = RistrettoPublicKeyBytes::from_bytes(revealed_receiver).map_err(|e| {
                OotleWasmError::InvalidByteLength {
                    field: "revealed_receiver",
                    expected: RistrettoPublicKeyBytes::length(),
                    got: e.actual_size(),
                }
            })?;
            Some(RevealedOutput::new(Amount::from_u64(amount), receiver))
        },
    };

    let statement = create_outputs_statement(witnesses.iter(), revealed_output)
        .map_err(|e| OotleWasmError::Stealth(e.to_string()))?;

    Ok(StealthOutputsResult {
        statement_json: serde_json::to_string(&statement)?,
        aggregated_output_mask: aggregated_output_mask.as_bytes().to_vec(),
    })
}

/// Inputs to [`create_stealth_output_witness`].
///
/// All keys are raw 32-byte slices; `resource_address` is the bech-style `resource_<hex>` string. The
/// commitment mask and ephemeral nonce are generated internally, so this struct carries only the
/// caller-controlled parameters.
pub struct CreateStealthOutputWitnessParams<'a> {
    /// Network byte (0x00 = MainNet, 0x10 = LocalNet, 0x26 = Esmeralda, ...).
    pub network: u8,
    /// Recipient's long-term account public key (used to derive the one-time stealth spend key).
    pub destination_account_public_key: &'a [u8],
    /// Recipient's view-only public key (used for the AEAD key and the UTXO scanning tag).
    pub destination_view_public_key: &'a [u8],
    /// Output value in microtari.
    pub amount: u64,
    /// The resource this output belongs to, as a `resource_<hex>` string.
    pub resource_address: &'a str,
    /// View public key of the resource view-key holder, if the resource has a viewable balance.
    pub resource_view_key: Option<&'a [u8]>,
    /// Optional JSON-encoded [`Memo`] to embed in the encrypted payload.
    pub memo_json: Option<&'a str>,
    /// Optional JSON-encoded [`PayTo`]: `"StealthPublicKey"` (the default when `None`) or
    /// `{"AccessRule": <AccessRule>}`.
    pub pay_to_json: Option<&'a str>,
    /// Minimum value the range proof commits to (must be `<= amount`). Use `0` unless you have a reason
    /// to reveal a lower bound.
    pub minimum_value_promise: u64,
}

/// Build a single stealth output witness entirely client-side, mirroring the wallet daemon's
/// `StealthOutputsApi::create_output_witness`.
///
/// Generates a fresh random commitment mask and ephemeral nonce, AEAD-encrypts the value+mask to the
/// recipient, derives the spend condition and the UTXO scanning tag, and returns one
/// [`StealthOutputWitnessJson`] serialized to a JSON string. Collect one such witness per output (including
/// change) into a JSON array and feed it to [`generate_stealth_outputs_statement`].
///
/// The mask is random rather than HD-derived: recipients (including the sender, for change) always recover
/// `value` and `mask` by decrypting `encrypted_data`, so determinism is not required.
pub fn create_stealth_output_witness(params: CreateStealthOutputWitnessParams<'_>) -> Result<String, OotleWasmError> {
    let network = Network::try_from(params.network).map_err(|e| OotleWasmError::InvalidNetwork(e.to_string()))?;
    let account_key = public_key_from_bytes(params.destination_account_public_key)?;
    let view_key = public_key_from_bytes(params.destination_view_public_key)?;
    let resource_address = ResourceAddress::from_str(params.resource_address)
        .map_err(|e| OotleWasmError::InvalidAddress(e.to_string()))?;
    let resource_view_key = params.resource_view_key.map(public_key_from_bytes).transpose()?;
    let memo: Option<Memo> = params.memo_json.map(serde_json::from_str).transpose()?;
    let pay_to: PayTo = params
        .pay_to_json
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();

    // Fail fast with a clear message rather than a cryptic range-proof error later, at statement time.
    if params.minimum_value_promise > params.amount {
        return Err(OotleWasmError::Stealth(format!(
            "minimum_value_promise ({}) must be <= amount ({})",
            params.minimum_value_promise, params.amount
        )));
    }

    let mask = RistrettoSecretKey::random(&mut rand::rng());
    let (nonce_secret, public_nonce) = RistrettoPublicKey::random_keypair(&mut rand::rng());

    let crypto = StealthCryptoApi::new();
    let encrypted_data = crypto
        .encrypt_value_and_mask(params.amount, &mask, &view_key, &nonce_secret, memo.as_ref())
        .map_err(|e| OotleWasmError::Stealth(e.to_string()))?;

    let auth = pay_to_output_authorization(&pay_to, || {
        crypto
            .derive_stealth_owner_public_key(network, &account_key, &view_key, &nonce_secret)
            .to_byte_type()
    })
    .map_err(|e| OotleWasmError::Stealth(e.to_string()))?;

    let tag = crypto.derive_stealth_output_tag(network, &nonce_secret, &view_key, &resource_address);

    let witness = StealthOutputWitness {
        witness: OutputWitness {
            amount: params.amount,
            mask,
            sender_public_nonce: public_nonce,
            minimum_value_promise: params.minimum_value_promise,
            encrypted_data,
            resource_view_key,
        },
        auth,
        tag,
    };

    Ok(serde_json::to_string(&StealthOutputWitnessJson::from(&witness))?)
}

/// Generate an extended bulletproof that aggregates range proofs for the supplied output witnesses.
///
/// Exposed independently of [`generate_stealth_outputs_statement`] for testing and audit. The Python
/// transfer flow uses the bundled variant.
///
/// `witnesses_json` is a JSON array of [`OutputWitnessJson`]. Returns the raw `RangeProofBytes` (may be
/// empty for an empty input array).
pub fn generate_extended_bullet_proof(witnesses_json: &str) -> Result<Vec<u8>, OotleWasmError> {
    let witnesses: Vec<OutputWitnessJson> = serde_json::from_str(witnesses_json)?;
    let witnesses = witnesses
        .into_iter()
        .map(OutputWitnessJson::try_into_witness)
        .collect::<Result<Vec<_>, OotleWasmError>>()?;

    let proof: RangeProofBytes =
        crypto_generate_extended_bullet_proof(witnesses.iter()).map_err(|e| OotleWasmError::Stealth(e.to_string()))?;
    Ok(proof.into_vec())
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{
        keys::{PublicKey, SecretKey},
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    };
    use tari_engine_types::stealth::validate_stealth_outputs_statement;
    use tari_template_lib_types::{EncryptedData, stealth::StealthOutputsStatement};

    use super::*;

    fn make_witness_json(amount: u64) -> String {
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let nonce = RistrettoPublicKey::from_secret_key(&mask);
        let spend_pk: tari_template_lib_types::crypto::RistrettoPublicKeyBytes = nonce.to_byte_type();
        format!(
            r#"[{{"witness":{{"amount":{},"mask":"{}","sender_public_nonce":"{}","minimum_value_promise":0,"encrypted_data":"{}"}},"auth":{{"Key":"{}"}},"tag":0}}]"#,
            amount,
            hex::encode(mask.as_bytes()),
            hex::encode(nonce.as_bytes()),
            hex::encode(vec![0u8; EncryptedData::min_size()]),
            hex::encode(spend_pk.as_bytes()),
        )
    }

    /// A stand-in receiver for a revealed output; the badge check that gives it meaning is the engine's.
    fn a_receiver() -> RistrettoPublicKeyBytes {
        let nonce = tari_crypto::ristretto::RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(5u64));
        nonce.to_byte_type()
    }

    #[test]
    fn generate_outputs_produces_valid_statement() {
        let witnesses = make_witness_json(1000);
        let result = generate_stealth_outputs_statement(&witnesses, 0, &[]).unwrap();
        let stmt: StealthOutputsStatement = serde_json::from_str(&result.statement_json).unwrap();
        validate_stealth_outputs_statement(&stmt, None).unwrap();
        assert_eq!(result.aggregated_output_mask.len(), 32);
    }

    #[test]
    fn generate_outputs_with_empty_array() {
        let result = generate_stealth_outputs_statement("[]", 100, a_receiver().as_bytes()).unwrap();
        let stmt: StealthOutputsStatement = serde_json::from_str(&result.statement_json).unwrap();
        assert!(stmt.outputs.is_empty());
        assert!(stmt.agg_range_proof.is_empty());
        assert_eq!(result.aggregated_output_mask, vec![0u8; 32]);
    }

    #[test]
    fn generate_extended_bullet_proof_empty() {
        let proof = generate_extended_bullet_proof("[]").unwrap();
        assert!(proof.is_empty());
    }

    #[test]
    fn generate_extended_bullet_proof_non_empty() {
        // Flat OutputWitnessJson (no auth / tag wrapper).
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let nonce = RistrettoPublicKey::from_secret_key(&mask);
        let witness_json = format!(
            r#"[{{"amount":50,"mask":"{}","sender_public_nonce":"{}","minimum_value_promise":0,"encrypted_data":"{}"}}]"#,
            hex::encode(mask.as_bytes()),
            hex::encode(nonce.as_bytes()),
            hex::encode(vec![0u8; EncryptedData::min_size()]),
        );
        let proof = generate_extended_bullet_proof(&witness_json).unwrap();
        assert!(!proof.is_empty());
    }

    fn tari_resource() -> String {
        tari_template_lib_types::constants::STEALTH_TARI_RESOURCE_ADDRESS.to_string()
    }

    fn random_public_key() -> RistrettoPublicKey {
        RistrettoPublicKey::random_keypair(&mut rand::rng()).1
    }

    #[test]
    fn created_witness_produces_valid_outputs_statement() {
        let account_pk = random_public_key();
        let view_pk = random_public_key();
        let resource = tari_resource();

        let json = create_stealth_output_witness(CreateStealthOutputWitnessParams {
            network: Network::LocalNet.as_byte(),
            destination_account_public_key: account_pk.as_bytes(),
            destination_view_public_key: view_pk.as_bytes(),
            amount: 1000,
            resource_address: &resource,
            resource_view_key: None,
            memo_json: None,
            pay_to_json: None,
            minimum_value_promise: 0,
        })
        .unwrap();

        let result = generate_stealth_outputs_statement(&format!("[{json}]"), 0, &[]).unwrap();
        let stmt: StealthOutputsStatement = serde_json::from_str(&result.statement_json).unwrap();
        validate_stealth_outputs_statement(&stmt, None).unwrap();
        assert_eq!(stmt.outputs.len(), 1);
        // The default pay_to produces a one-time stealth spend key (key path, no condition tree).
        assert!(stmt.outputs[0].auth.spend_key().is_some());
        assert!(stmt.outputs[0].auth.condition_root().is_none());
    }

    #[test]
    fn created_witness_is_spendable_and_decryptable() {
        use tari_crypto::commitment::HomomorphicCommitmentFactory;
        use tari_engine_types::crypto::get_commitment_factory;

        let network = Network::LocalNet;
        let (account_sk, account_pk) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (view_sk, view_pk) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let amount = 12_345u64;
        let resource = tari_resource();

        let json = create_stealth_output_witness(CreateStealthOutputWitnessParams {
            network: network.as_byte(),
            destination_account_public_key: account_pk.as_bytes(),
            destination_view_public_key: view_pk.as_bytes(),
            amount,
            resource_address: &resource,
            resource_view_key: None,
            memo_json: None,
            pay_to_json: None,
            minimum_value_promise: 0,
        })
        .unwrap();

        let witness: StealthOutputWitness = serde_json::from_str::<StealthOutputWitnessJson>(&json)
            .unwrap()
            .try_into()
            .unwrap();

        // The recipient recomputes the AEAD key from (view_secret, sender_public_nonce) and decrypts the
        // value + mask back out, confirming the output is addressed to them.
        let commitment = get_commitment_factory()
            .commit_value(&witness.witness.mask, amount)
            .to_byte_type();
        let encryption_key = crate::stealth::kdfs::encrypted_data_dh_kdf(
            view_sk.as_bytes(),
            witness.witness.sender_public_nonce.as_bytes(),
        )
        .unwrap();
        let decrypted = crate::stealth::encrypted_data::unblind_output(
            commitment.as_bytes(),
            witness.witness.encrypted_data.as_bytes(),
            &encryption_key,
            false,
        )
        .unwrap();
        assert_eq!(decrypted.value, amount);
        assert_eq!(decrypted.mask, witness.witness.mask.as_bytes().to_vec());

        // The recipient's one-time spend secret matches the spend condition's public key, so the output
        // is actually spendable.
        let stealth_secret_bytes = crate::stealth::kdfs::stealth_dh_secret(
            network.as_byte(),
            account_sk.as_bytes(),
            view_sk.as_bytes(),
            witness.witness.sender_public_nonce.as_bytes(),
        )
        .unwrap();
        let stealth_secret = RistrettoSecretKey::from_canonical_bytes(&stealth_secret_bytes).unwrap();
        let derived_pub = RistrettoPublicKey::from_secret_key(&stealth_secret);
        let expected = witness.auth.spend_key().expect("expected a key-path spend_key");
        assert_eq!(derived_pub.to_byte_type(), *expected);
    }

    #[test]
    fn created_witness_with_access_rule() {
        let account_pk = random_public_key();
        let view_pk = random_public_key();
        let resource = tari_resource();

        let json = create_stealth_output_witness(CreateStealthOutputWitnessParams {
            network: Network::LocalNet.as_byte(),
            destination_account_public_key: account_pk.as_bytes(),
            destination_view_public_key: view_pk.as_bytes(),
            amount: 500,
            resource_address: &resource,
            resource_view_key: None,
            memo_json: None,
            pay_to_json: Some(r#"{"AccessRule":"AllowAll"}"#),
            minimum_value_promise: 0,
        })
        .unwrap();

        let witness: StealthOutputWitnessJson = serde_json::from_str(&json).unwrap();
        // An AccessRule pay_to produces a single-leaf condition tree (a condition_root, no key path).
        assert!(witness.auth.spend_key().is_none());
        assert!(witness.auth.condition_root().is_some());

        let result = generate_stealth_outputs_statement(&format!("[{json}]"), 0, &[]).unwrap();
        let stmt: StealthOutputsStatement = serde_json::from_str(&result.statement_json).unwrap();
        validate_stealth_outputs_statement(&stmt, None).unwrap();
    }

    #[test]
    fn created_witness_with_memo_round_trips() {
        use tari_crypto::commitment::HomomorphicCommitmentFactory;
        use tari_engine_types::crypto::get_commitment_factory;

        let (_, account_pk) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (view_sk, view_pk) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let amount = 777u64;
        let resource = tari_resource();

        let json = create_stealth_output_witness(CreateStealthOutputWitnessParams {
            network: Network::LocalNet.as_byte(),
            destination_account_public_key: account_pk.as_bytes(),
            destination_view_public_key: view_pk.as_bytes(),
            amount,
            resource_address: &resource,
            resource_view_key: None,
            memo_json: Some(r#"{"Message":"gm"}"#),
            pay_to_json: None,
            minimum_value_promise: 0,
        })
        .unwrap();

        let witness: StealthOutputWitness = serde_json::from_str::<StealthOutputWitnessJson>(&json)
            .unwrap()
            .try_into()
            .unwrap();
        let commitment = get_commitment_factory()
            .commit_value(&witness.witness.mask, amount)
            .to_byte_type();
        let encryption_key = crate::stealth::kdfs::encrypted_data_dh_kdf(
            view_sk.as_bytes(),
            witness.witness.sender_public_nonce.as_bytes(),
        )
        .unwrap();
        let decrypted = crate::stealth::encrypted_data::unblind_output(
            commitment.as_bytes(),
            witness.witness.encrypted_data.as_bytes(),
            &encryption_key,
            false,
        )
        .unwrap();
        assert_eq!(decrypted.memo_json.as_deref(), Some(r#"{"Message":"gm"}"#));
    }

    #[test]
    fn create_witness_rejects_invalid_resource_address() {
        let account_pk = random_public_key();
        let view_pk = random_public_key();

        let err = create_stealth_output_witness(CreateStealthOutputWitnessParams {
            network: Network::LocalNet.as_byte(),
            destination_account_public_key: account_pk.as_bytes(),
            destination_view_public_key: view_pk.as_bytes(),
            amount: 1,
            resource_address: "not-a-resource",
            resource_view_key: None,
            memo_json: None,
            pay_to_json: None,
            minimum_value_promise: 0,
        })
        .unwrap_err();
        assert!(matches!(err, OotleWasmError::InvalidAddress(_)));
    }

    #[test]
    fn create_witness_rejects_minimum_value_promise_above_amount() {
        let account_pk = random_public_key();
        let view_pk = random_public_key();
        let resource = tari_resource();

        let err = create_stealth_output_witness(CreateStealthOutputWitnessParams {
            network: Network::LocalNet.as_byte(),
            destination_account_public_key: account_pk.as_bytes(),
            destination_view_public_key: view_pk.as_bytes(),
            amount: 100,
            resource_address: &resource,
            resource_view_key: None,
            memo_json: None,
            pay_to_json: None,
            minimum_value_promise: 101,
        })
        .unwrap_err();
        assert!(matches!(err, OotleWasmError::Stealth(_)));
    }
}
