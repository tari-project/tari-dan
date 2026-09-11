//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::iter;

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_ootle_wallet_crypto::{
    MaskAndValue,
    OutputWitness,
    StealthInputWitness,
    StealthOutputWitness,
    balance_proof::generate_stealth_balance_proof_signature,
    stealth,
};
use tari_template_lib::types::{
    Amount,
    EncryptedData,
    bytes::Bytes,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
    stealth::{SpendAuthorization, SpendCondition, StealthOutputsStatement, StealthTransferStatement},
};

use crate::support::spec::{InputAuthSpec, InputSpec, OutputAuthSpec, OutputSpec};

/// The resolved authorisation of a generated stealth output, recorded so a later transfer can reconstruct the spend
/// witness for it.
#[derive(Debug, Clone)]
pub enum OutputAuth {
    /// Key path with this resolved `spend_key`.
    KeyPath(RistrettoPublicKeyBytes),
    /// Condition tree over these committed leaves.
    Conditions(Vec<SpendCondition>),
}

pub fn generate_stealth_output_statement<I: IntoIterator<Item = u64>, A: Into<Amount>>(
    output_amounts: I,
    revealed_output_amount: A,
) -> (StealthOutputsStatement, Vec<RistrettoSecretKey>) {
    generate_stealth_statement_internal(
        &output_amounts.into_iter().collect::<Vec<_>>(),
        revealed_output_amount.into(),
        None,
    )
}

pub fn generate_mint_statement<I: IntoIterator<Item = OS>, OS: Into<OutputSpec>, A: Into<Amount> + Copy>(
    stealth_output_amounts: I,
    revealed_output_amount: A,
    view_key: Option<&RistrettoPublicKey>,
) -> StealthSecretTransferData {
    let stealth_output_amounts = stealth_output_amounts.into_iter().map(Into::into).collect::<Vec<_>>();
    let total_revealed_inputs = stealth_output_amounts
        .iter()
        .map(|os| Amount::from(os.value()))
        .sum::<Amount>() +
        revealed_output_amount.into();
    match view_key {
        Some(view_key) => generate_transfer_data_with_view_key(
            iter::empty::<InputSpec>(),
            total_revealed_inputs,
            stealth_output_amounts,
            revealed_output_amount.into(),
            view_key,
        ),

        None => generate_transfer_data(
            iter::empty::<InputSpec>(),
            total_revealed_inputs,
            stealth_output_amounts,
            revealed_output_amount.into(),
        ),
    }
}

pub fn generate_stealth_statement_with_view_key<I: IntoIterator<Item = u64>>(
    output_amounts: I,
    revealed_output_amount: Amount,
    view_key: &RistrettoPublicKey,
) -> (StealthOutputsStatement, Vec<RistrettoSecretKey>) {
    generate_stealth_statement_internal(
        &output_amounts.into_iter().collect::<Vec<_>>(),
        revealed_output_amount,
        Some(view_key.clone()),
    )
}

fn generate_stealth_statement_internal(
    output_amounts: &[u64],
    revealed_output_amount: Amount,
    view_key: Option<RistrettoPublicKey>,
) -> (StealthOutputsStatement, Vec<RistrettoSecretKey>) {
    let masks = output_amounts
        .iter()
        .map(|_| RistrettoSecretKey::random(&mut rand::rng()))
        .collect::<Vec<_>>();
    let output_statements = output_amounts
        .iter()
        .zip(&masks)
        .map(|(amount, mask)| StealthOutputWitness {
            witness: OutputWitness {
                amount: *amount,
                mask: mask.clone(),
                sender_public_nonce: test_sender_public_nonce(),
                minimum_value_promise: 0,
                encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
                resource_view_key: view_key.clone(),
            },
            // Default: a key-path output owned by the mask's public key.
            auth: SpendAuthorization::Key(RistrettoPublicKey::from_secret_key(mask).to_byte_type()),
            tag: UtxoTag::new(0),
        })
        .collect::<Vec<_>>();

    let stmt = stealth::create_outputs_statement(&output_statements, revealed_output_amount).unwrap();
    (stmt, masks)
}

/// Rewrites `data` to spend its first input twice, re-signing the balance proof over the doubled input set.
///
/// `stealth::create_transfer_statement` refuses to build this, so it is assembled here: the engine has to reject a
/// double-spent input on its own terms rather than rely on the submitter having used a well-behaved builder. The
/// spender knows the input mask, so the proof over the doubled set is theirs to construct.
pub fn spend_first_input_twice(
    data: &StealthSecretTransferData,
    input_mask: &RistrettoSecretKey,
) -> StealthTransferStatement {
    let mut statement = data.statement.clone();
    let first = statement.inputs_statement.inputs[0].clone();
    statement.inputs_statement.inputs.push(first);

    let agg_input_mask = input_mask.clone() + input_mask;
    let agg_output_mask = data
        .output_masks
        .iter()
        .fold(RistrettoSecretKey::default(), |agg, mask| agg + mask);
    statement.balance_proof = Some(generate_stealth_balance_proof_signature(
        &agg_input_mask,
        &agg_output_mask,
        &statement.inputs_statement,
        &statement.outputs_statement,
    ));

    statement
}

pub struct StealthSecretTransferData {
    pub output_masks: Vec<RistrettoSecretKey>,
    /// The resolved authorisation of each generated output (parallel to `output_masks`), so a later transfer can build
    /// the witness to spend it.
    pub output_auths: Vec<OutputAuth>,
    pub statement: StealthTransferStatement,
}

impl StealthSecretTransferData {
    /// Builds an [`InputSpec`] spending output `i` (worth `value`) of this transfer, using the recorded output auth.
    /// Condition-tree outputs reveal their first committed leaf (single-leaf trees, the common case in tests).
    pub fn input_spec_for(&self, i: usize, value: u64) -> InputSpec {
        let mask_and_value = MaskAndValue {
            mask: self.output_masks[i].clone(),
            value,
        };
        match &self.output_auths[i] {
            OutputAuth::KeyPath(_) => InputSpec::new(mask_and_value),
            OutputAuth::Conditions(conditions) => InputSpec::with_auth(mask_and_value, InputAuthSpec::ScriptPath {
                conditions: conditions.clone(),
                leaf: conditions[0].clone(),
                data: Bytes::default(),
            }),
        }
    }
}

pub const NO_INPUTS: iter::Empty<MaskAndValue> = iter::empty();
pub fn generate_transfer_data<O, A, OS, IS, II>(
    inputs: II,
    revealed_input_amount: A,
    outputs: O,
    revealed_output_amount: A,
) -> StealthSecretTransferData
where
    O: IntoIterator<Item = OS>,
    OS: Into<OutputSpec>,
    A: Into<Amount>,
    II: IntoIterator<Item = IS>,
    II::IntoIter: ExactSizeIterator,
    IS: Into<InputSpec>,
{
    generate_transfer_data_internal(inputs, revealed_input_amount, outputs, revealed_output_amount, None)
}

pub fn generate_transfer_data_with_view_key<IO, OS, A, II, IS>(
    inputs: II,
    revealed_input_amount: A,
    outputs: IO,
    revealed_output_amount: A,
    view_key: &RistrettoPublicKey,
) -> StealthSecretTransferData
where
    IO: IntoIterator<Item = OS>,
    OS: Into<OutputSpec>,
    A: Into<Amount>,
    II: IntoIterator<Item = IS>,
    II::IntoIter: ExactSizeIterator,
    IS: Into<InputSpec>,
{
    generate_transfer_data_internal(
        inputs,
        revealed_input_amount,
        outputs,
        revealed_output_amount,
        Some(view_key.clone()),
    )
}

/// Generates a non-zero test sender nonce keypair for testing purposes.
/// This is not secure (it is a hardcoded value) and should only be used for testing scenarios.
pub fn test_sender_nonce_keypair() -> (RistrettoSecretKey, RistrettoPublicKey) {
    let sender_nonce = RistrettoSecretKey::from(123);
    let sender_public_nonce = RistrettoPublicKey::from_secret_key(&sender_nonce);
    (sender_nonce, sender_public_nonce)
}
pub fn test_sender_public_nonce() -> RistrettoPublicKey {
    test_sender_nonce_keypair().1
}

fn generate_transfer_data_internal<IO, OS, A, II, IS>(
    inputs: II,
    revealed_input_amount: A,
    outputs: IO,
    revealed_output_amount: A,
    view_key: Option<RistrettoPublicKey>,
) -> StealthSecretTransferData
where
    IO: IntoIterator<Item = OS>,
    OS: Into<OutputSpec>,
    A: Into<Amount>,
    II: IntoIterator<Item = IS>,
    II::IntoIter: ExactSizeIterator,
    IS: Into<InputSpec>,
{
    let mut output_auths = Vec::new();
    let outputs = outputs
        .into_iter()
        .map(Into::into)
        .filter(|os| os.value() > 0)
        .map(|spec| {
            let output_mask = RistrettoSecretKey::random(&mut rand::rng());
            let statement = OutputWitness {
                amount: spec.value(),
                mask: output_mask.clone(),
                resource_view_key: view_key.clone(),
                // This is client/wallet on-chain data and not required for spending in tests
                sender_public_nonce: test_sender_public_nonce(),
                minimum_value_promise: 0,
                encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
            };

            let (authorization, auth) = match spec.auth() {
                OutputAuthSpec::KeyPathFromMask => {
                    // For testing purposes, we use the mask as the owner key
                    let pk = RistrettoPublicKey::from_secret_key(&output_mask).to_byte_type();
                    (SpendAuthorization::Key(pk), OutputAuth::KeyPath(pk))
                },
                OutputAuthSpec::KeyPath(pk) => (SpendAuthorization::Key(*pk), OutputAuth::KeyPath(*pk)),
                OutputAuthSpec::Conditions(conditions) => {
                    let root = stealth::condition_root(conditions).unwrap();
                    (
                        SpendAuthorization::Script(root),
                        OutputAuth::Conditions(conditions.clone()),
                    )
                },
                OutputAuthSpec::KeyAndConditions { spend_key, conditions } => {
                    let root = stealth::condition_root(conditions).unwrap();
                    (
                        SpendAuthorization::KeyAndScript {
                            spend_key: *spend_key,
                            condition_root: root,
                        },
                        // The exploit reclaims this output via its key path, so record the key-path auth.
                        OutputAuth::KeyPath(*spend_key),
                    )
                },
            };
            output_auths.push(auth);

            StealthOutputWitness {
                witness: statement,
                auth: authorization,
                tag: UtxoTag::new(0),
            }
        })
        .collect::<Vec<_>>();

    let inputs = inputs
        .into_iter()
        .map(Into::into)
        .map(|input: InputSpec| match input.auth() {
            InputAuthSpec::KeyPath => StealthInputWitness::new(input.mask_and_value().clone()),
            InputAuthSpec::ScriptPath { conditions, leaf, data } => {
                let (witness, root) = stealth::script_path_witness_with_data(conditions, leaf, data.clone()).unwrap();
                StealthInputWitness::with_script_path(input.mask_and_value().clone(), witness, root)
            },
        });

    let transfer = stealth::create_transfer_statement(
        inputs,
        revealed_input_amount.into(),
        outputs.iter(),
        revealed_output_amount.into(),
    )
    .unwrap();

    StealthSecretTransferData {
        output_masks: outputs.into_iter().map(|m| m.witness.mask).collect(),
        output_auths,
        statement: transfer,
    }
}
