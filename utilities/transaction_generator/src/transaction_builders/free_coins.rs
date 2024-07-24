//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};
use tari_engine_types::{component::new_component_address_from_public_key, instruction::Instruction};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    constants::{XTR_FAUCET_COMPONENT_ADDRESS, XTR_FAUCET_VAULT_ADDRESS},
    models::Amount,
};
use tari_transaction::{SubstateRequirement, Transaction};

pub fn builder(_: u64) -> Transaction {
    let (signer_secret_key, signer_public_key) = RistrettoPublicKey::random_keypair(&mut OsRng);

    let account_address = new_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &signer_public_key);

    Transaction::builder()
        .with_fee_instructions_builder(|builder| {
            builder
                .add_instruction(Instruction::CallMethod {
                    component_address: XTR_FAUCET_COMPONENT_ADDRESS,
                    method: "take".to_string(),
                    args: args![Amount(5000)],
                })
                .put_last_instruction_output_on_workspace(b"free_coins")
                .create_account_with_bucket(signer_public_key, "free_coins")
                .call_method(account_address, "pay_fee", args![Amount(1000)])
        })
        .with_inputs([
            SubstateRequirement::unversioned(XTR_FAUCET_COMPONENT_ADDRESS),
            SubstateRequirement::unversioned(XTR_FAUCET_VAULT_ADDRESS),
        ])
        .sign(&signer_secret_key)
        .build()
}
