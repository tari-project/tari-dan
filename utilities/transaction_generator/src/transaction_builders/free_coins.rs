//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_ootle_common_types::{Epoch, InputDeclaration};
use tari_ootle_transaction::{Network, Transaction, args};
use tari_template_lib_types::constants::{
    TARI_TOKEN,
    XTR_FAUCET_CLAIM_RESOURCE_ADDRESS,
    XTR_FAUCET_COMPONENT_ADDRESS,
    XTR_FAUCET_VAULT_ADDRESS,
};

use super::accounts::account_keypair;

pub fn builder(network: Network, max_epoch: Epoch) -> impl Fn(u64) -> Transaction {
    move |_: u64| -> Transaction {
        let (signer_secret_key, signer_public_key) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        claim(network, max_epoch, &signer_secret_key, &signer_public_key)
    }
}

/// Claims from the faucet into the accounts derived from `seed`, transaction `i` funding account `i`.
///
/// The faucet pays each account once, so these are the accounts a [`transfers`](super::transfers) flood with the
/// same seed spends from.
pub fn seeded_builder(network: Network, max_epoch: Epoch, seed: u64) -> impl Fn(u64) -> Transaction {
    move |index: u64| -> Transaction {
        let (signer_secret_key, signer_public_key) = account_keypair(seed, index);
        claim(network, max_epoch, &signer_secret_key, &signer_public_key)
    }
}

fn claim(
    network: Network,
    max_epoch: Epoch,
    signer_secret_key: &RistrettoSecretKey,
    signer_public_key: &RistrettoPublicKey,
) -> Transaction {
    let signer_public_key = signer_public_key.to_byte_type();

    Transaction::builder(network.as_byte(), max_epoch)
        .with_fee_instructions_builder(|builder| {
            builder
                .create_account(signer_public_key)
                .put_last_instruction_output_on_workspace("account")
                .call_method(XTR_FAUCET_COMPONENT_ADDRESS, "take", args![Workspace("account")])
                // The fee intent creates the account and claims into it, so it pays for a
                // component, a vault, a badge and the receipt. Allocated well above what that
                // costs — the excess refunds, and a tight value silently stops generating
                // transactions whenever pricing moves.
                .call_method("account", "pay_fee", args![50_000])
        })
        .with_inputs([
            InputDeclaration::read(TARI_TOKEN),
            InputDeclaration::write(XTR_FAUCET_COMPONENT_ADDRESS),
            InputDeclaration::write(XTR_FAUCET_VAULT_ADDRESS),
            InputDeclaration::read(XTR_FAUCET_CLAIM_RESOURCE_ADDRESS),
        ])
        .build_and_seal(signer_secret_key)
}
