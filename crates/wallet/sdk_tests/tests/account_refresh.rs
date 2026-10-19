//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Refreshing an account reads its component and then every vault it holds, each proven when the wallet verifies its
//! indexer.

mod support;

use tari_engine_types::{
    component::{Component, ComponentBody, ComponentHeader},
    resource::Resource,
    resource_container::ResourceContainer,
    substate::{SubstateId, SubstateValue},
    vault::Vault,
};
use tari_ootle_common_types::{Epoch, SubstateVersion};
use tari_ootle_light_client::test_fixtures::{ValidatorSetFixture, substate_proof};
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    models::{KeyBranch, KeyId},
};
use tari_ootle_wallet_sdk_services::{account_monitor::AccountScanner, notify::Notify};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    Metadata,
    ResourceAddress,
    ResourceType,
    SubstateOwnerRule,
    VaultId,
    access_rules::{ComponentAccessRules, ResourceAccessRules},
};

use crate::support::{ProvingNetwork, Test, TestSdkSpec, TestWithNetwork};

const VERSION: SubstateVersion = SubstateVersion::new(3);

fn resource_address() -> ResourceAddress {
    "resource_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff"
        .parse()
        .unwrap()
}

fn vault_id(seed: u8) -> VaultId {
    VaultId::from_hex(&format!("{seed:02x}").repeat(32)).unwrap()
}

fn account_value(vaults: &[VaultId]) -> SubstateValue {
    Component {
        header: ComponentHeader {
            template_address: ACCOUNT_TEMPLATE_ADDRESS,
            owner_rule: SubstateOwnerRule::None,
            access_rules: ComponentAccessRules::new(),
            entity_id: Default::default(),
        },
        body: ComponentBody::from_cbor_value(tari_bor::to_value(&vaults.to_vec()).unwrap()),
    }
    .into()
}

fn vault_value(amount: u64) -> SubstateValue {
    Vault::new(ResourceContainer::public_fungible(resource_address(), amount)).into()
}

/// A wallet holding the test account, which has walked to the validator set of epoch 3, and the network serving it.
async fn verifying_wallet() -> (
    Vec<ValidatorSetFixture>,
    ProvingNetwork,
    TestWithNetwork<ProvingNetwork>,
) {
    let sets = (1..=3).map(ValidatorSetFixture::new).collect::<Vec<_>>();
    let network = ProvingNetwork::new(&sets, 1);
    let test = TestWithNetwork::without_accounts_trusting(network.clone(), IndexerTrust::Verify);
    let trusted = test.sdk().validator_set_api().sync().await.unwrap();
    assert_eq!(trusted.epoch(), Epoch(3));

    test.sdk()
        .accounts_api()
        .add_account(
            Some("test"),
            &account_address(),
            KeyId::derived(KeyBranch::ViewOnlyKey, 0),
            KeyId::derived(KeyBranch::Account, 0),
            Epoch::zero(),
            true,
            true,
        )
        .unwrap();
    test.sdk()
        .resources_api()
        .upsert_resource(
            &resource_address(),
            &Resource::new(
                ResourceType::Fungible,
                SubstateOwnerRule::None,
                ResourceAccessRules::new(),
                Metadata::new(),
                None,
                None,
                6,
                false,
            ),
        )
        .unwrap();
    (sets, network, test)
}

fn account_address() -> ComponentAddress {
    Test::test_account_address()
}

fn serve_proven(sets: &[ValidatorSetFixture], network: &ProvingNetwork, id: SubstateId, value: SubstateValue) {
    let proof = substate_proof(sets.last().unwrap(), &id, VERSION, &value);
    network.serve_substate(id, VERSION, value, Some(proof));
}

fn scanner(test: &TestWithNetwork<ProvingNetwork>) -> AccountScanner<TestSdkSpec<ProvingNetwork>> {
    AccountScanner::new(Notify::new(1), test.sdk().clone())
}

#[tokio::test]
async fn an_account_refresh_reads_all_its_vaults_in_one_fetch() {
    let (sets, network, test) = verifying_wallet().await;
    let vaults = (1..=5).map(vault_id).collect::<Vec<_>>();
    serve_proven(&sets, &network, account_address().into(), account_value(&vaults));
    for (i, vault) in vaults.iter().enumerate() {
        serve_proven(&sets, &network, (*vault).into(), vault_value(100 * (i as u64 + 1)));
    }

    assert!(scanner(&test).refresh_account(account_address()).await.unwrap());

    assert_eq!(network.substate_queries(), 1, "the account is read alone");
    assert_eq!(network.substate_fetches(), 1, "the vaults are read in one fetch");
    for (i, vault) in vaults.iter().enumerate() {
        let stored = test.sdk().accounts_api().get_vault(vault).unwrap();
        assert_eq!(stored.revealed_balance, Amount::from(100 * (i as u64 + 1)));
    }
}

#[tokio::test]
async fn an_account_refresh_skips_a_vault_the_indexer_does_not_serve() {
    let (sets, network, test) = verifying_wallet().await;
    let served = vault_id(1);
    let missing = vault_id(2);
    serve_proven(
        &sets,
        &network,
        account_address().into(),
        account_value(&[missing, served]),
    );
    serve_proven(&sets, &network, served.into(), vault_value(100));

    assert!(scanner(&test).refresh_account(account_address()).await.unwrap());

    assert_eq!(
        test.sdk().accounts_api().get_vault(&served).unwrap().revealed_balance,
        Amount::from(100u64)
    );
    assert!(!test.sdk().accounts_api().has_vault(&missing).unwrap());
}

#[tokio::test]
async fn an_account_refresh_leaves_up_to_date_vaults_alone() {
    let (sets, network, test) = verifying_wallet().await;
    let vault = vault_id(1);
    serve_proven(&sets, &network, account_address().into(), account_value(&[vault]));
    serve_proven(&sets, &network, vault.into(), vault_value(100));
    assert!(scanner(&test).refresh_account(account_address()).await.unwrap());

    assert!(!scanner(&test).refresh_account(account_address()).await.unwrap());
}
