//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use ootle_byte_type::ToByteType;
use tari_crypto::commitment::HomomorphicCommitmentFactory;
use tari_engine_types::{
    confidential_output::ConfidentialOutput,
    crypto::{OutputBody, get_commitment_factory},
    substate::{Substate, SubstateDiff, SubstateId},
};
use tari_ootle_common_types::SubstateVersion;
use tari_ootle_wallet_sdk::{
    models::{ConfidentialOutputModel, KeyBranch, KeyId, OutputStatus},
    storage::{ReadableWalletStore, WalletStoreReader},
};
use tari_template_lib::types::{ConfidentialOutputAddress, EncryptedData, constants::STEALTH_TARI_RESOURCE_ADDRESS};

use crate::support::Test;

#[test]
fn outputs_locked_and_released() {
    let test = Test::new();

    let commitment_25 = test.add_unspent_output(25u64);
    let commitment_49 = test.add_unspent_output(49u64);
    let _commitment_100 = test.add_unspent_output(100u64);

    let lock_id = test.new_lock().keep_locked();
    let (inputs, total_value) = test
        .sdk()
        .confidential_outputs_api()
        .lock_outputs_by_amount(lock_id, &Test::test_vault_address(), 50u64)
        .unwrap();
    assert_eq!(total_value, 74);
    assert_eq!(inputs.len(), 2);

    let locked = test
        .store()
        .with_read_tx(|tx| tx.confidential_outputs_get_locked_by_lock_id(lock_id))
        .unwrap();

    assert!(locked.iter().any(|l| l.commitment == commitment_25));
    assert!(locked.iter().any(|l| l.commitment == commitment_49));
    assert_eq!(locked.len(), 2);

    test.sdk().locks_api().release_lock(lock_id).unwrap();

    let locked = test
        .store()
        .with_read_tx(|tx| tx.confidential_outputs_get_locked_by_lock_id(lock_id))
        .unwrap();
    assert_eq!(locked.len(), 0);
}

#[test]
fn outputs_locked_and_finalized() {
    let test = Test::new();

    let commitment_25 = test.add_unspent_output(25u64);
    let commitment_49 = test.add_unspent_output(49u64);
    let commitment_100 = test.add_unspent_output(100u64);

    let outputs_api = test.sdk().confidential_outputs_api();
    let locks_api = test.sdk().locks_api();
    let lock_id = test.new_lock().keep_locked();

    let (inputs, total_value) = outputs_api
        .lock_outputs_by_amount(lock_id, &Test::test_vault_address(), 50u64)
        .unwrap();
    assert_eq!(total_value, 74);
    assert_eq!(inputs.len(), 2);

    let locked = test
        .store()
        .with_read_tx(|tx| tx.confidential_outputs_get_locked_by_lock_id(lock_id))
        .unwrap();

    assert!(locked.iter().any(|l| l.commitment == commitment_25));
    assert!(locked.iter().any(|l| l.commitment == commitment_49));
    assert_eq!(locked.len(), 2);

    // Add a change output belonging to this proof
    let commitment_change = get_commitment_factory()
        .commit_value(&Default::default(), 24)
        .to_byte_type();
    outputs_api
        .add_output(ConfidentialOutputModel {
            account_address: Test::test_account_address(),
            vault_id: Test::test_vault_address(),
            commitment: commitment_change,
            value: 24u64.into(),
            sender_public_nonce: None,
            view_only_key_id: KeyId::derived(KeyBranch::ViewOnlyKey, 0),
            owner_key_id: Some(KeyId::derived(KeyBranch::Account, 0)),
            encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
            public_asset_tag: None,
            memo: None,
            status: OutputStatus::LockedUnconfirmed,
            lock_id: Some(lock_id),
        })
        .unwrap();

    let balance = test.get_unspent_balance();
    assert_eq!(balance, 100);

    let output_id = |commitment| {
        SubstateId::ConfidentialOutput(ConfidentialOutputAddress::new(
            STEALTH_TARI_RESOURCE_ADDRESS,
            commitment,
        ))
    };
    let mut diff = SubstateDiff::new();
    diff.down(output_id(commitment_25), SubstateVersion::ZERO);
    diff.down(output_id(commitment_49), SubstateVersion::ZERO);
    diff.up(
        output_id(commitment_change),
        Substate::new(SubstateVersion::ZERO, ConfidentialOutput {
            output: OutputBody {
                public_nonce: Default::default(),
                encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
                minimum_value_promise: 0,
                viewable_balance: None,
            },
            is_frozen: false,
        }),
    );
    locks_api.finalize_lock(lock_id, &diff).unwrap();

    {
        let mut tx = test.store().create_read_tx().unwrap();
        let locked = tx.confidential_outputs_get_locked_by_lock_id(lock_id).unwrap();
        assert_eq!(locked.len(), 0);

        let unspent = tx
            .confidential_outputs_get_by_account_and_status(&Test::test_account_address(), OutputStatus::Unspent)
            .unwrap();
        assert!(unspent.iter().any(|l| l.commitment == commitment_change));
        assert!(unspent.iter().any(|l| l.commitment == commitment_100));
        assert_eq!(unspent.len(), 2);
        let balance = tx
            .confidential_outputs_get_unspent_balance(&Test::test_vault_address())
            .unwrap();
        assert_eq!(balance, 124);
    }
}
