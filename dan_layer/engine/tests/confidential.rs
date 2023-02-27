//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_common_types::types::PrivateKey;
use tari_crypto::keys::SecretKey;
use tari_dan_engine::crypto;
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress},
    prelude::ConfidentialProof,
};
use tari_template_test_tooling::{SubstateType, TemplateTest};

fn setup(initial_supply: ConfidentialProof) -> (TemplateTest, ComponentAddress, SubstateAddress) {
    let mut template_test = TemplateTest::new(vec!["tests/templates/confidential/faucet"]);

    let faucet: ComponentAddress =
        template_test.call_function("ConfidentialFaucet", "mint", args![initial_supply], vec![]);

    let resx = template_test.get_previous_output_address(SubstateType::Resource);

    (template_test, faucet, resx)
}

fn generate_confidential_proof(amount: Amount) -> (ConfidentialProof, PrivateKey) {
    let mask = PrivateKey::random(&mut OsRng);
    // If value is negative we have a massive u64 - perfect!
    let proof = crypto::generate_confidential_proof(&mask, amount.value() as u64, 0);
    (proof, mask)
}
#[test]
fn mint_initial_commitment() {
    let (confidential_proof, _mask) = generate_confidential_proof(Amount(100));
    let (mut template_test, faucet, _faucet_resx) = setup(confidential_proof);

    let total_supply: Amount = template_test.call_method(faucet, "total_supply", args![], vec![]);
    // The number of commitments
    assert_eq!(total_supply, Amount(1));
}

#[test]
#[ignore = "Confidential withdraw is not yet implemented"]
fn transfer_confidential_amounts_between_accounts() {
    let (confidential_proof, _mask) = generate_confidential_proof(Amount(100));
    let (mut template_test, faucet, _faucet_resx) = setup(confidential_proof);

    let total_supply: Amount = template_test.call_method(faucet, "total_supply", args![], vec![]);
    // The number of commitments
    assert_eq!(total_supply, Amount(1));

    // Create an account
    let (account1, _owner1, _k) = template_test.create_owned_account();
    let (_account2, _owner2, _k) = template_test.create_owned_account();

    // Transfer faucet funds into account 1
    let vars = [("faucet", faucet.into()), ("account1", account1.into())];
    let result = template_test
        .execute_and_commit_manifest(
            r#"
        let faucet = var!["faucet"];
        let account1 = var!["account1"];
        let coins = faucet.take_free_coins();
        account1.deposit(coins);
    "#,
            vars,
            vec![],
        )
        .unwrap();
    let _diff = result.result.expect("Failed to execute manifest");
}
