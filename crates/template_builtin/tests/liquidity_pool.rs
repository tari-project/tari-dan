//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use tari_engine_types::indexed_value::IndexedWellKnownTypes;
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::{
    models::ComponentAddressAllocation,
    types::{AccessRule, OwnerRule, constants::TARI_TOKEN, metadata},
};
use tari_template_lib_types::{Amount, ComponentAddress, ResourceAddress, access_rules::ResourceAuthAction};
use tari_template_test_tooling::{
    TemplateTest,
    support::assert_error::{assert_access_denied_for_action, assert_reject_reason},
};

const TEMPLATE_NAME: &str = "TwoResourceLiquidityPool";
const TEMPLATE_PATHS: &[&str] = &["tests/faucet", "tests/resource_minter"];
const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn initial_contribution_and_redeem() {
    // Setup
    // NOTE: builtin templates are automatically added to the test environment. No need to specify template paths.
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template_address = test.get_template_address(TEMPLATE_NAME);

    let (faucet_component, faucet_resource) = create_test_faucet_component(&mut test, 1_000_000_000_000u64);

    let (user1, _user1_proof, user1_secret) = test.create_funded_account();

    // ACT 1: Create liquidity pool and contribute liquidity
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .allocate_component_address("pool")
            .call_function(template_address, "create", args![
                OwnerRule::OwnedBySigner,
                AccessRule::AllowAll,
                TARI_TOKEN,
                faucet_resource,
                metadata!["name" => "XTR-Stablecoin Liquidity Pool"],
                Workspace("pool"),
            ])
            .call_method(faucet_component, "take_free_coins_custom", args![2000])
            .put_last_instruction_output_on_workspace("faucet_coins")
            .call_method(user1, "withdraw", args![TARI_TOKEN, 1000])
            .put_last_instruction_output_on_workspace("xtr_coins")
            .call_method("pool", "contribute", args![
                Workspace("xtr_coins"),
                Workspace("faucet_coins")
            ])
            .put_last_instruction_output_on_workspace("contribution")
            .call_method(user1, "deposit", args![Workspace("contribution.0")])
            .call_method(user1, "deposit", args![Workspace("contribution.1")])
            .call_method(user1, "deposit", args![Workspace("contribution.2")])
            .finish()
            .add_signer(&test.to_public_key_bytes(), &user1_secret)
            .seal(test.secret_key()),
        vec![],
    );

    // Assert ACT 1
    let store = test.read_only_state_store();
    let (pool_addr, pool_body) = store
        .get_components_by_template_address(template_address)
        .unwrap()
        .pop()
        .unwrap();
    let indexed = pool_body.body.to_indexed_well_known_types().unwrap();
    let vaults = indexed
        .vault_ids()
        .iter()
        .map(|id| store.get_vault(id).unwrap())
        .map(|v| (*v.resource_address(), v))
        .collect::<HashMap<_, _>>();
    let pool_unit_resx = indexed.resource_addresses().first().copied().unwrap();

    assert_eq!(vaults.get(&TARI_TOKEN).unwrap().balance(), 1000);
    assert_eq!(vaults.get(&faucet_resource).unwrap().balance(), 2000);

    let user_account = store.get_account(user1).unwrap();
    let vault = user_account.get_vault_by_resource(&pool_unit_resx).unwrap();
    let pool_units = store.get_vault(&vault.vault_id()).unwrap();
    assert!(pool_units.balance() >= 1000);

    // ACT 2: Redeem liquidity
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(user1, "withdraw", args![pool_unit_resx, pool_units.balance()])
            .put_last_instruction_output_on_workspace("redeem_pool_units")
            .call_method(pool_addr, "redeem", args![Workspace("redeem_pool_units")])
            .put_last_instruction_output_on_workspace("redeemed_coins")
            .assert_bucket_contains_at_least("redeemed_coins.0", TARI_TOKEN, 1000u64)
            .assert_bucket_contains_at_least("redeemed_coins.1", faucet_resource, 2000u64)
            .call_method(user1, "deposit", args![Workspace("redeemed_coins.0")])
            .call_method(user1, "deposit", args![Workspace("redeemed_coins.1")])
            .finish()
            .seal(&user1_secret),
        vec![],
    );

    // Assert ACT 2
    let store = test.read_only_state_store();
    let user_account = store.get_account(user1).unwrap();
    let xtr_vault = user_account.get_vault_by_resource(&TARI_TOKEN).unwrap();
    let xtr_coins = store.get_vault(&xtr_vault.vault_id()).unwrap();
    assert_eq!(xtr_coins.balance(), TemplateTest::FUNDED_ACCOUNT_INITIAL_BALANCE);
    let stablecoin_vault = user_account.get_vault_by_resource(&faucet_resource).unwrap();
    let stablecoin_coins = store.get_vault(&stablecoin_vault.vault_id()).unwrap();
    assert!(stablecoin_coins.balance() >= 2000);
}

#[test]
fn only_the_pool_can_mint_or_burn_lp_tokens() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template_address = test.get_template_address(TEMPLATE_NAME);
    let minter_template = test.get_template_address("ResourceMinter");

    let (faucet_component, faucet_resource) = create_test_faucet_component(&mut test, 1_000_000_000_000u64);
    let (provider, _provider_proof, provider_secret) = test.create_funded_account();
    let (attacker, _attacker_proof, attacker_secret) = test.create_funded_account();

    // A public pool with no address allocation, so the pool allocates its own address for the LP rules. The pool is
    // owned by a third key, because the resource owner may mint and burn regardless of the rules.
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_address, "create", args![
                OwnerRule::OwnedBySigner,
                AccessRule::AllowAll,
                TARI_TOKEN,
                faucet_resource,
                metadata!["name" => "TARI-Stablecoin Liquidity Pool"],
                None::<ComponentAddressAllocation>,
            ])
            .finish()
            .seal(test.secret_key()),
        vec![],
    );
    let store = test.read_only_state_store();
    let (pool_addr, pool_body) = store
        .get_components_by_template_address(template_address)
        .unwrap()
        .pop()
        .unwrap();
    let indexed = pool_body.body.to_indexed_well_known_types().unwrap();
    let lp_resx = indexed.resource_addresses().first().copied().unwrap();

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet_component, "take_free_coins_custom", args![2000])
            .put_last_instruction_output_on_workspace("faucet_coins")
            .call_method(provider, "withdraw", args![TARI_TOKEN, 1000])
            .put_last_instruction_output_on_workspace("xtr_coins")
            .call_method(pool_addr, "contribute", args![
                Workspace("xtr_coins"),
                Workspace("faucet_coins")
            ])
            .put_last_instruction_output_on_workspace("contribution")
            .call_method(provider, "deposit", args![Workspace("contribution.0")])
            .call_method(provider, "deposit", args![Workspace("contribution.1")])
            .call_method(provider, "deposit", args![Workspace("contribution.2")])
            .finish()
            .seal(&provider_secret),
        vec![],
    );

    // Minting LP outside the pool and redeeming it would drain the reserves.
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(minter_template, "mint", args![lp_resx, 1_000_000_000])
            .put_last_instruction_output_on_workspace("unbacked_lp")
            .call_method(pool_addr, "redeem", args![Workspace("unbacked_lp")])
            .put_last_instruction_output_on_workspace("stolen")
            .call_method(attacker, "deposit", args![Workspace("stolen.0")])
            .call_method(attacker, "deposit", args![Workspace("stolen.1")])
            .finish()
            .seal(&attacker_secret),
        vec![],
    );
    assert_access_denied_for_action(reason, ResourceAuthAction::Mint);

    // Burning LP outside the pool would leave the reserves backing a smaller supply.
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(provider, "withdraw", args![lp_resx, 100])
            .put_last_instruction_output_on_workspace("lp")
            .call_function(minter_template, "burn", args![Workspace("lp")])
            .finish()
            .seal(&provider_secret),
        vec![],
    );
    assert_access_denied_for_action(reason, ResourceAuthAction::Burn);

    // The pool itself still burns LP on redeem.
    let store = test.read_only_state_store();
    let provider_account = store.get_account(provider).unwrap();
    let lp_vault = provider_account.get_vault_by_resource(&lp_resx).unwrap();
    let lp_balance = store.get_vault(&lp_vault.vault_id()).unwrap().balance();
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(provider, "withdraw", args![lp_resx, lp_balance])
            .put_last_instruction_output_on_workspace("lp")
            .call_method(pool_addr, "redeem", args![Workspace("lp")])
            .put_last_instruction_output_on_workspace("redeemed")
            .assert_bucket_contains_at_least("redeemed.0", TARI_TOKEN, 1000u64)
            .assert_bucket_contains_at_least("redeemed.1", faucet_resource, 2000u64)
            .call_method(provider, "deposit", args![Workspace("redeemed.0")])
            .call_method(provider, "deposit", args![Workspace("redeemed.1")])
            .finish()
            .seal(&provider_secret),
        vec![],
    );
}

#[test]
fn second_contribution_and_partial_redeem() {
    // Regression test for the `(true, true, true)` contribute branch and partial redemption, neither of which were
    // previously covered. A non-proportional second contribution exercises (a) the ratio math that must not truncate
    // to zero and (b) the change bucket returned for the over-supplied resource. The partial redeem exercises the
    // proportional-payout math that must not truncate to zero either.
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template_address = test.get_template_address(TEMPLATE_NAME);

    let (faucet_component, faucet_resource) = create_test_faucet_component(&mut test, 1_000_000_000_000u64);
    let (user1, _user1_proof, user1_secret) = test.create_funded_account();

    // ACT 1: create the pool with an initial 1000 TARI : 4000 stablecoin contribution (ratio 1:4, LP minted = 2000).
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .allocate_component_address("pool")
            .call_function(template_address, "create", args![
                OwnerRule::OwnedBySigner,
                AccessRule::AllowAll,
                TARI_TOKEN,
                faucet_resource,
                metadata!["name" => "TARI-Stablecoin Liquidity Pool"],
                Workspace("pool"),
            ])
            .call_method(faucet_component, "take_free_coins_custom", args![4000])
            .put_last_instruction_output_on_workspace("faucet_coins")
            .call_method(user1, "withdraw", args![TARI_TOKEN, 1000])
            .put_last_instruction_output_on_workspace("xtr_coins")
            .call_method("pool", "contribute", args![
                Workspace("xtr_coins"),
                Workspace("faucet_coins")
            ])
            .put_last_instruction_output_on_workspace("contribution")
            .call_method(user1, "deposit", args![Workspace("contribution.0")])
            .call_method(user1, "deposit", args![Workspace("contribution.1")])
            .call_method(user1, "deposit", args![Workspace("contribution.2")])
            .finish()
            .add_signer(&test.to_public_key_bytes(), &user1_secret)
            .seal(test.secret_key()),
        vec![],
    );

    let store = test.read_only_state_store();
    let (pool_addr, pool_body) = store
        .get_components_by_template_address(template_address)
        .unwrap()
        .pop()
        .unwrap();
    let indexed = pool_body.body.to_indexed_well_known_types().unwrap();
    let lp_resx = indexed.resource_addresses().first().copied().unwrap();

    // ACT 2: a deliberately non-proportional second contribution: 500 TARI but 4000 stablecoin. At the 1:4 reserve
    // ratio only 2000 stablecoin is needed to match the 500 TARI, so 2000 stablecoin must be returned as change.
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet_component, "take_free_coins_custom", args![4000])
            .put_last_instruction_output_on_workspace("faucet_coins")
            .call_method(user1, "withdraw", args![TARI_TOKEN, 500])
            .put_last_instruction_output_on_workspace("xtr_coins")
            .call_method(pool_addr, "contribute", args![
                Workspace("xtr_coins"),
                Workspace("faucet_coins")
            ])
            .put_last_instruction_output_on_workspace("contribution")
            // change_a (TARI) is empty, change_b (stablecoin) must hold the unused 2000.
            .assert_bucket_contains_at_least("contribution.2", faucet_resource, 2000u64)
            .call_method(user1, "deposit", args![Workspace("contribution.0")])
            .call_method(user1, "deposit", args![Workspace("contribution.1")])
            .call_method(user1, "deposit", args![Workspace("contribution.2")])
            .finish()
            .seal(&user1_secret),
        vec![],
    );

    // Reserves must keep the 1:4 ratio: a = 1000 + 500 = 1500, b = 4000 + 2000 = 6000.
    let store = test.read_only_state_store();
    let pool_body = store.get_component(pool_addr).unwrap();
    let indexed = pool_body.body.to_indexed_well_known_types().unwrap();
    let vaults = indexed
        .vault_ids()
        .iter()
        .map(|id| store.get_vault(id).unwrap())
        .map(|v| (*v.resource_address(), v))
        .collect::<HashMap<_, _>>();
    assert_eq!(vaults.get(&TARI_TOKEN).unwrap().balance(), 1500);
    assert_eq!(vaults.get(&faucet_resource).unwrap().balance(), 6000);

    // The user should now hold LP = 2000 (initial) + 1000 (second) = 3000.
    let user_account = store.get_account(user1).unwrap();
    let lp_vault = user_account.get_vault_by_resource(&lp_resx).unwrap();
    let lp_balance = store.get_vault(&lp_vault.vault_id()).unwrap().balance();
    assert_eq!(lp_balance, 3000);

    // ACT 3: redeem HALF the LP (1500 of 3000). Proportional payout = 750 TARI and 3000 stablecoin.
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(user1, "withdraw", args![lp_resx, 1500])
            .put_last_instruction_output_on_workspace("redeem_lp")
            .call_method(pool_addr, "redeem", args![Workspace("redeem_lp")])
            .put_last_instruction_output_on_workspace("redeemed")
            .assert_bucket_contains_at_least("redeemed.0", TARI_TOKEN, 750u64)
            .assert_bucket_contains_at_least("redeemed.1", faucet_resource, 3000u64)
            .call_method(user1, "deposit", args![Workspace("redeemed.0")])
            .call_method(user1, "deposit", args![Workspace("redeemed.1")])
            .finish()
            .seal(&user1_secret),
        vec![],
    );

    // Half the reserves should remain in the pool: a = 750, b = 3000.
    let store = test.read_only_state_store();
    let pool_body = store.get_component(pool_addr).unwrap();
    let indexed = pool_body.body.to_indexed_well_known_types().unwrap();
    let vaults = indexed
        .vault_ids()
        .iter()
        .map(|id| store.get_vault(id).unwrap())
        .map(|v| (*v.resource_address(), v))
        .collect::<HashMap<_, _>>();
    assert_eq!(vaults.get(&TARI_TOKEN).unwrap().balance(), 750);
    assert_eq!(vaults.get(&faucet_resource).unwrap().balance(), 3000);
}

struct PoolState {
    reserve_tari: u64,
    reserve_other: u64,
    lp_supply: u64,
}

fn read_pool_state(test: &TemplateTest, pool_addr: ComponentAddress, other: ResourceAddress) -> PoolState {
    let store = test.read_only_state_store();
    let pool_body = store.get_component(pool_addr).unwrap();
    let indexed = pool_body.body.to_indexed_well_known_types().unwrap();
    let lp_resx = indexed.resource_addresses().first().copied().unwrap();
    let vaults = indexed
        .vault_ids()
        .iter()
        .map(|id| store.get_vault(id).unwrap())
        .map(|v| (*v.resource_address(), v.balance().to_u64_checked().unwrap()))
        .collect::<HashMap<_, _>>();
    let lp_supply = store
        .get_resource(&lp_resx)
        .unwrap()
        .total_supply()
        .unwrap()
        .to_u64_checked()
        .unwrap();
    PoolState {
        reserve_tari: vaults[&TARI_TOKEN],
        reserve_other: vaults[&other],
        lp_supply,
    }
}

/// Seeds a pool with reserves (3 TARI, 1_000_000 faucet), which mints 1732 LP, then contributes `(tari, other)`.
/// A reserve of a few base units of A makes the B value of one unit of A large, so any rounding in the pool's
/// favour on the B side is easy to observe.
fn contribute_to_thin_pool(tari: u64, other: u64) -> (PoolState, PoolState) {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template_address = test.get_template_address(TEMPLATE_NAME);
    let (faucet_component, faucet_resource) = create_test_faucet_component(&mut test, 1_000_000_000_000u64);
    let (user1, _user1_proof, user1_secret) = test.create_funded_account();

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .allocate_component_address("pool")
            .call_function(template_address, "create", args![
                OwnerRule::OwnedBySigner,
                AccessRule::AllowAll,
                TARI_TOKEN,
                faucet_resource,
                metadata!["name" => "TARI-Stablecoin Liquidity Pool"],
                Workspace("pool"),
            ])
            .call_method(faucet_component, "take_free_coins_custom", args![1_000_000u64])
            .put_last_instruction_output_on_workspace("faucet_coins")
            .call_method(user1, "withdraw", args![TARI_TOKEN, 3u64])
            .put_last_instruction_output_on_workspace("xtr_coins")
            .call_method("pool", "contribute", args![
                Workspace("xtr_coins"),
                Workspace("faucet_coins")
            ])
            .put_last_instruction_output_on_workspace("contribution")
            .call_method(user1, "deposit", args![Workspace("contribution.0")])
            .call_method(user1, "deposit", args![Workspace("contribution.1")])
            .call_method(user1, "deposit", args![Workspace("contribution.2")])
            .finish()
            .add_signer(&test.to_public_key_bytes(), &user1_secret)
            .seal(test.secret_key()),
        vec![],
    );

    let (pool_addr, _) = test
        .read_only_state_store()
        .get_components_by_template_address(template_address)
        .unwrap()
        .pop()
        .unwrap();
    let before = read_pool_state(&test, pool_addr, faucet_resource);
    assert_eq!(before.reserve_tari, 3);
    assert_eq!(before.reserve_other, 1_000_000);
    assert_eq!(before.lp_supply, 1732);

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet_component, "take_free_coins_custom", args![other])
            .put_last_instruction_output_on_workspace("faucet_coins")
            .call_method(user1, "withdraw", args![TARI_TOKEN, tari])
            .put_last_instruction_output_on_workspace("xtr_coins")
            .call_method(pool_addr, "contribute", args![
                Workspace("xtr_coins"),
                Workspace("faucet_coins")
            ])
            .put_last_instruction_output_on_workspace("contribution")
            .call_method(user1, "deposit", args![Workspace("contribution.0")])
            .call_method(user1, "deposit", args![Workspace("contribution.1")])
            .call_method(user1, "deposit", args![Workspace("contribution.2")])
            .finish()
            .seal(&user1_secret),
        vec![],
    );

    let after = read_pool_state(&test, pool_addr, faucet_resource);
    (before, after)
}

/// The value backing each LP token must not fall for existing holders, and the pool must take no more of either
/// resource than the minted LP is worth (rounded up by less than one base unit).
#[track_caller]
fn assert_contribution_is_proportional(before: &PoolState, after: &PoolState) {
    let minted = u128::from(after.lp_supply - before.lp_supply);
    let lp = u128::from(before.lp_supply);
    for (reserve_before, reserve_after) in [
        (before.reserve_tari, after.reserve_tari),
        (before.reserve_other, after.reserve_other),
    ] {
        let reserve_before = u128::from(reserve_before);
        let taken = u128::from(reserve_after) - reserve_before;
        let owed = minted * reserve_before;
        assert!(taken * lp >= owed, "pool took less than the minted LP is worth");
        assert!(
            taken * lp < owed + lp,
            "pool took {taken} for {minted} LP, more than one unit above its value"
        );
    }
}

#[test]
fn contribute_returns_the_excess_of_both_buckets() {
    // Surplus B: the A bucket limits the mint, and only the B that mint is worth is taken.
    let (before, after) = contribute_to_thin_pool(1, 400_000);
    assert_eq!(after.lp_supply - before.lp_supply, 577);
    assert_eq!(after.reserve_tari - before.reserve_tari, 1);
    assert_eq!(after.reserve_other - before.reserve_other, 333_141);
    assert_contribution_is_proportional(&before, &after);

    // Surplus A: the B bucket limits the mint, and the remainder of both buckets is returned.
    let (before, after) = contribute_to_thin_pool(10, 400_000);
    assert_eq!(after.lp_supply - before.lp_supply, 692);
    assert_eq!(after.reserve_tari - before.reserve_tari, 2);
    assert_eq!(after.reserve_other - before.reserve_other, 399_539);
    assert_contribution_is_proportional(&before, &after);
}

#[test]
fn owner_adds_liquidity_only_after_bootstrap() {
    // Liquidity added through the owner-only `protected_add_liquidity` mints no LP, so it accrues to the existing LP
    // holders. A pool with no LP outstanding is funded only through `contribute`.
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template_address = test.get_template_address(TEMPLATE_NAME);

    let (faucet_component, faucet_resource) = create_test_faucet_component(&mut test, 1_000_000_000_000u64);
    let (user1, user1_proof, user1_secret) = test.create_funded_account();

    // ACT 1: create the pool owned by user1 (so user1 can call the owner-gated protected_* methods). With
    // `OwnerRule::OwnedBySigner` the owner is the seal signer, i.e. user1.
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .allocate_component_address("pool")
            .call_function(template_address, "create", args![
                OwnerRule::OwnedBySigner,
                AccessRule::AllowAll,
                TARI_TOKEN,
                faucet_resource,
                metadata!["name" => "TARI-Stablecoin Liquidity Pool"],
                Workspace("pool"),
            ])
            .build_and_seal(&user1_secret),
        vec![user1_proof.clone()],
    );

    let store = test.read_only_state_store();
    let (pool_addr, _) = store
        .get_components_by_template_address(template_address)
        .unwrap()
        .pop()
        .unwrap();

    // ACT 2: the owner cannot seed a reserve while there is no LP supply.
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(user1, "withdraw", args![TARI_TOKEN, 1000])
            .put_last_instruction_output_on_workspace("seed_a")
            .call_method(pool_addr, "protected_add_liquidity", args![Workspace("seed_a")])
            .build_and_seal(&user1_secret),
        vec![user1_proof.clone()],
    );
    assert_reject_reason(&reason, "Cannot add liquidity to a pool with no LP supply");

    // ACT 3: bootstrap with 500 TARI + 2000 stablecoin (LP minted = floor(sqrt(500 * 2000)) = 1000), then top up
    // reserve A with 1000 TARI, which accrues to the LP holders. Reserves become (1500, 2000).
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet_component, "take_free_coins_custom", args![2000])
            .put_last_instruction_output_on_workspace("faucet_coins")
            .call_method(user1, "withdraw", args![TARI_TOKEN, 500])
            .put_last_instruction_output_on_workspace("xtr_coins")
            .call_method(pool_addr, "contribute", args![
                Workspace("xtr_coins"),
                Workspace("faucet_coins")
            ])
            .put_last_instruction_output_on_workspace("contribution")
            .call_method(user1, "deposit", args![Workspace("contribution.0")])
            .call_method(user1, "deposit", args![Workspace("contribution.1")])
            .call_method(user1, "deposit", args![Workspace("contribution.2")])
            .call_method(user1, "withdraw", args![TARI_TOKEN, 1000])
            .put_last_instruction_output_on_workspace("top_up_a")
            .call_method(pool_addr, "protected_add_liquidity", args![Workspace("top_up_a")])
            .build_and_seal(&user1_secret),
        vec![user1_proof.clone()],
    );

    let store = test.read_only_state_store();
    let pool_body = store.get_component(pool_addr).unwrap();
    let indexed = pool_body.body.to_indexed_well_known_types().unwrap();
    let vaults = indexed
        .vault_ids()
        .iter()
        .map(|id| store.get_vault(id).unwrap())
        .map(|v| (*v.resource_address(), v))
        .collect::<HashMap<_, _>>();
    assert_eq!(vaults.get(&TARI_TOKEN).unwrap().balance(), 1500);
    assert_eq!(vaults.get(&faucet_resource).unwrap().balance(), 2000);

    let lp_resx = indexed.resource_addresses().first().copied().unwrap();
    let user_account = store.get_account(user1).unwrap();
    let lp_vault = user_account.get_vault_by_resource(&lp_resx).unwrap();
    let lp_balance = store.get_vault(&lp_vault.vault_id()).unwrap().balance();
    assert_eq!(lp_balance, 1000);
}

#[test]
fn swap_is_rejected_while_a_reserve_is_empty() {
    // A pool with an empty reserve has no price, so a swap in either direction must be refused.
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template_address = test.get_template_address(TEMPLATE_NAME);

    let (faucet_component, faucet_resource) = create_test_faucet_component(&mut test, 1_000_000_000_000u64);
    let (owner, owner_proof, owner_secret) = test.create_funded_account();
    let (trader, _trader_proof, trader_secret) = test.create_funded_account();

    const RESERVE_A: u64 = 1_000_000;
    const RESERVE_B: u64 = 1_000;

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .allocate_component_address("pool")
            .call_function(template_address, "create", args![
                OwnerRule::OwnedBySigner,
                AccessRule::AllowAll,
                TARI_TOKEN,
                faucet_resource,
                metadata!["name" => "TARI-Stablecoin Liquidity Pool"],
                Workspace("pool"),
            ])
            .build_and_seal(&owner_secret),
        vec![owner_proof.clone()],
    );

    let store = test.read_only_state_store();
    let (pool_addr, _) = store
        .get_components_by_template_address(template_address)
        .unwrap()
        .pop()
        .unwrap();

    // The owner bootstraps the pool, then withdraws all of reserve B.
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet_component, "take_free_coins_custom", args![RESERVE_B])
            .put_last_instruction_output_on_workspace("faucet_coins")
            .call_method(owner, "withdraw", args![TARI_TOKEN, RESERVE_A])
            .put_last_instruction_output_on_workspace("xtr_coins")
            .call_method(pool_addr, "contribute", args![
                Workspace("xtr_coins"),
                Workspace("faucet_coins")
            ])
            .put_last_instruction_output_on_workspace("contribution")
            .call_method(owner, "deposit", args![Workspace("contribution.0")])
            .call_method(owner, "deposit", args![Workspace("contribution.1")])
            .call_method(owner, "deposit", args![Workspace("contribution.2")])
            .call_method(pool_addr, "protected_remove_liquidity", args![
                faucet_resource,
                RESERVE_B
            ])
            .put_last_instruction_output_on_workspace("removed")
            .call_method(owner, "deposit", args![Workspace("removed")])
            .build_and_seal(&owner_secret),
        vec![owner_proof],
    );

    // Swapping into the empty side.
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet_component, "take_free_coins_custom", args![1])
            .put_last_instruction_output_on_workspace("input")
            .call_method(pool_addr, "swap", args![Workspace("input")])
            .put_last_instruction_output_on_workspace("output")
            .call_method(trader, "deposit", args![Workspace("output")])
            .build_and_seal(&trader_secret),
        vec![],
    );
    assert_reject_reason(&reason, "Pool has no liquidity");

    // Swapping out of the empty side.
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(trader, "withdraw", args![TARI_TOKEN, 1000])
            .put_last_instruction_output_on_workspace("input")
            .call_method(pool_addr, "swap", args![Workspace("input")])
            .put_last_instruction_output_on_workspace("output")
            .call_method(trader, "deposit", args![Workspace("output")])
            .build_and_seal(&trader_secret),
        vec![],
    );
    assert_reject_reason(&reason, "Pool has no liquidity");

    let store = test.read_only_state_store();
    let pool_body = store.get_component(pool_addr).unwrap();
    let indexed = pool_body.body.to_indexed_well_known_types().unwrap();
    let vaults = indexed
        .vault_ids()
        .iter()
        .map(|id| store.get_vault(id).unwrap())
        .map(|v| (*v.resource_address(), v))
        .collect::<HashMap<_, _>>();
    assert_eq!(vaults.get(&TARI_TOKEN).unwrap().balance(), RESERVE_A);
    assert_eq!(vaults.get(&faucet_resource).unwrap().balance(), 0);
}

#[test]
fn basic_constant_product_swap() {
    // Setup
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template_address = test.get_template_address(TEMPLATE_NAME);

    let (faucet_component, faucet_resource) = create_test_faucet_component(&mut test, 1_000_000_000_000u64);

    let (user1, _user1_proof, user1_secret) = test.create_funded_account();
    let (user2, _user2_proof, user2_secret) = test.create_empty_account();

    const INITIAL_XTR_AMOUNT: u64 = 1_000_000;
    const INITIAL_STABLECOIN_AMOUNT: u64 = 2_000_000;

    // ACT 1: Create liquidity pool and contribute liquidity
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .allocate_component_address("pool")
            .call_function(template_address, "create", args![
                OwnerRule::OwnedBySigner,
                AccessRule::AllowAll,
                TARI_TOKEN,
                faucet_resource,
                metadata!["name" => "XTR-Stablecoin Liquidity Pool"],
                Workspace("pool"),
            ])
            .call_method(faucet_component, "take_free_coins_custom", args![
                INITIAL_STABLECOIN_AMOUNT
            ])
            .put_last_instruction_output_on_workspace("faucet_coins")
            .call_method(user1, "withdraw", args![TARI_TOKEN, INITIAL_XTR_AMOUNT])
            .put_last_instruction_output_on_workspace("xtr_coins")
            .call_method("pool", "contribute", args![
                Workspace("xtr_coins"),
                Workspace("faucet_coins")
            ])
            .put_last_instruction_output_on_workspace("contribution")
            .call_method(user1, "deposit", args![Workspace("contribution.0")])
            .call_method(user1, "deposit", args![Workspace("contribution.1")])
            .call_method(user1, "deposit", args![Workspace("contribution.2")])
            // Give user some stablecoin for later
            .call_method(faucet_component, "take_free_coins_custom", args![
                1_000_000
            ])
            .put_last_instruction_output_on_workspace("user2_stablecoin")
            .call_method(user2, "deposit", args![Workspace("user2_stablecoin")])
            .finish()
            .add_signer(&test.to_public_key_bytes(), &user1_secret)
            .seal(test.secret_key()),
        vec![],
    );

    let store = test.read_only_state_store();
    let (pool_addr, _) = store
        .get_components_by_template_address(template_address)
        .unwrap()
        .pop()
        .unwrap();

    // ACT 2: Swap to pay fees
    test.enable_fees();

    // The swap has to return more XTR than the whole transaction costs — it is the only XTR user2
    // ever holds — and the fee payment has to cover that cost out of what the swap returned.
    const SWAP_AMOUNT: u64 = 20_000;
    const FEE_PAYMENT: u64 = 5_000;

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| {
                // User2 would like to swap stablecoin for XTR to pay fees
                builder
                    .call_method(user2, "withdraw", args![faucet_resource, SWAP_AMOUNT])
                    .put_last_instruction_output_on_workspace("coins_to_swap")
                    .call_method(pool_addr, "swap_constant_product", args![Workspace("coins_to_swap")])
                    .put_last_instruction_output_on_workspace("swapped_coins")
                    .call_method(user2, "deposit", args![Workspace("swapped_coins")])
                    .pay_fee_from_component(user2, FEE_PAYMENT)
            })
            // Some instruction to make fees
            .call_method(user2, "create_proof_by_amount", args![faucet_resource, 1])
            .put_last_instruction_output_on_workspace("_proof")
            .drop_all_proofs_in_workspace()
            .finish()
            .seal(&user2_secret),
        vec![],
    );

    // Assert ACT 2
    let store = test.read_only_state_store();
    let user_account = store.get_account(user2).unwrap();
    let xtr_vault = user_account.get_vault_by_resource(&TARI_TOKEN).unwrap();
    let xtr_coins = store.get_vault(&xtr_vault.vault_id()).unwrap();
    let fees = result.finalize.fee_receipt.total_fees_paid();
    assert!(fees > 0);

    /// We duplicate the formula here to verify the template calculation.
    /// NOTE: this will not work for all values due to rounding errors.
    fn constant_product_calc(x_balance: u64, y_balance: u64, delta_x: u64) -> u64 {
        let x_balance = u128::from(x_balance);
        let y_balance = u128::from(y_balance);
        let delta_x = u128::from(delta_x);
        let delta_y = (y_balance * delta_x) / (x_balance + delta_x);
        u64::try_from(delta_y).unwrap()
    }

    assert_eq!(
        xtr_coins.balance(),
        constant_product_calc(INITIAL_STABLECOIN_AMOUNT, INITIAL_XTR_AMOUNT, SWAP_AMOUNT) - fees
    );
}

#[track_caller]
pub fn create_test_faucet_component<A: Into<Amount>>(
    test: &mut TemplateTest,
    initial_supply: A,
) -> (ComponentAddress, ResourceAddress) {
    let template_addr = test.get_template_address("TestFaucet");
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "mint", args![initial_supply.into()])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let (addr, component) = result
        .expect_success()
        .up_iter()
        .filter_map(|(id, substate)| {
            id.as_component_address().and_then(|addr| {
                let component = substate.substate_value().as_component()?;
                if *component.template_address() == template_addr {
                    Some((addr, component.clone()))
                } else {
                    None
                }
            })
        })
        .next()
        .expect("No component address found in faucet creation result");

    let indexed = IndexedWellKnownTypes::from_value(component.state()).unwrap();
    let vault_id = indexed
        .vault_ids()
        .first()
        .expect("No vault id found in faucet component state");
    let vault = test
        .read_only_state_store()
        .get_vault(vault_id)
        .expect("No vault id found in faucet component state");
    (addr, *vault.resource_address())
}
