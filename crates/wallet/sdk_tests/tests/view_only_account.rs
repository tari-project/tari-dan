//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use ootle_byte_type::ToByteType;
use tari_engine_types::{Utxo, UtxoOutput, crypto::OutputBody};
use tari_ootle_common_types::Epoch;
use tari_ootle_wallet_sdk::{
    crypto::{memo::Memo, pay_to::PayTo},
    models::{KeyBranch, KeyId, OutputStatus},
};
use tari_template_lib::types::{UtxoAddress, constants::STEALTH_TARI_RESOURCE_ADDRESS};

use crate::support::{PanicNetworkInterface, Test};

#[test]
fn view_only_account_decrypts_incoming_stealth_output() {
    let sender = Test::new();
    let cold = Test::without_accounts(PanicNetworkInterface);
    let watch = Test::without_accounts(PanicNetworkInterface);

    let cold_keys = cold.sdk().key_manager_api().derive_account_address(0).unwrap();
    let cold_address = cold_keys.address;
    let view_secret_key = cold
        .sdk()
        .key_manager_api()
        .get_key(KeyId::derived(KeyBranch::ViewOnlyKey, 0))
        .unwrap()
        .secret;

    let account = watch
        .sdk()
        .accounts_api()
        .add_view_only_account(
            Some("watch"),
            &cold_address.account_key.to_byte_type(),
            &view_secret_key,
            Some(Epoch::zero()),
            true,
        )
        .unwrap();
    assert_eq!(account.owner_key_id(), None);
    assert_eq!(*account.address(), cold_address.to_byte_type());

    let sender_address = sender
        .sdk()
        .key_manager_api()
        .derive_account_address(0)
        .unwrap()
        .address;
    let memo = Memo::new_sender_address(
        sender_address.account_key.to_byte_type().into_array(),
        sender_address.view_only_key.to_byte_type().into_array(),
        b"bridge-uuid",
    )
    .unwrap();
    let output = sender
        .sdk()
        .stealth_outputs_api()
        .create_output_witness(
            &cold_address,
            1234,
            &STEALTH_TARI_RESOURCE_ADDRESS,
            None,
            Some(&memo),
            PayTo::StealthPublicKey,
        )
        .unwrap();
    let commitment = output.witness.to_commitment().to_byte_type();
    let utxo = Utxo::new(UtxoOutput {
        output: OutputBody {
            public_nonce: output.witness.sender_public_nonce.to_byte_type(),
            encrypted_data: output.witness.encrypted_data.clone(),
            minimum_value_promise: 0,
            viewable_balance: None,
        },
        auth: output.auth,
        tag: output.tag,
    });

    watch
        .sdk()
        .stealth_outputs_api()
        .verify_and_update_outputs([(
            UtxoAddress::new(STEALTH_TARI_RESOURCE_ADDRESS, commitment.into()),
            &utxo,
        )])
        .unwrap();

    let outputs = watch
        .sdk()
        .stealth_outputs_api()
        .utxos_get_many(&STEALTH_TARI_RESOURCE_ADDRESS, Some(account.component_address()), None)
        .unwrap();
    assert_eq!(outputs.len(), 1);
    let output = &outputs[0];
    assert_eq!(output.value, 1234);
    assert!(matches!(output.status, OutputStatus::Unspent), "{:?}", output.status);
    assert_eq!(output.owner_key_id, None);
    let (sender_account_key, sender_view_key, pay_ref) = output
        .memo
        .as_ref()
        .and_then(|m| m.as_sender_address())
        .expect("sender address memo");
    assert_eq!(sender_account_key, sender_address.account_key.to_byte_type().as_bytes());
    assert_eq!(sender_view_key, sender_address.view_only_key.to_byte_type().as_bytes());
    assert_eq!(pay_ref, b"bridge-uuid");
}
