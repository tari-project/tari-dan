//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tari_engine_types::{component::new_component_address_from_parts, instruction::Instruction};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, NonFungibleAddress},
};
use tari_transaction::Transaction;

pub fn builder(_: u64) -> Transaction {
    let (signer_secret_key, signer_public_key) = RistrettoPublicKey::random_keypair(&mut OsRng);

    let owner_pk = RistrettoPublicKeyBytes::from_bytes(signer_public_key.as_bytes()).unwrap();
    let owner_token = NonFungibleAddress::from_public_key(owner_pk);
    Transaction::builder()
        .with_fee_instructions_builder(|builder| {
            builder
                .add_instruction(Instruction::CreateFreeTestCoins {
                    revealed_amount: Amount::new(1000),
                    output: None,
                })
                .put_last_instruction_output_on_workspace(b"free_coins")
                .call_function(ACCOUNT_TEMPLATE_ADDRESS, "create_with_bucket", args![
                    owner_token,
                    Workspace("free_coins")
                ])
                .call_method(
                    new_component_address_from_parts(&ACCOUNT_TEMPLATE_ADDRESS, &owner_pk.into_array().into()),
                    "pay_fee",
                    args![Amount(1000)],
                )
        })
        .sign(&signer_secret_key)
        .build()
}
