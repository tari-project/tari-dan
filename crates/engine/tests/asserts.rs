//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::vec;

use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine::runtime::{AssertError, RuntimeError};
use tari_ootle_transaction::{Assertion, CheckOrd, Epoch, Instruction, Transaction, args, args::WorkspaceOffsetId};
use tari_template_lib::types::{
    ComponentAddress,
    NonFungibleAddress,
    NonFungibleId,
    ResourceAddress,
    ResourceType,
    constants::{NFT_FAUCET_COMPONENT_ADDRESS, NFT_FAUCET_RESOURCE_ADDRESS, TARI_TOKEN},
};
use tari_template_test_tooling::{TemplateTest, support::assert_error::assert_reject_reason};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const TEMPLATE_PATH: &str = "tests/templates/tariswap";

const FAUCET_WITHDRAWAL_AMOUNT: u32 = 1000;

struct AssertTest {
    template_test: TemplateTest,
    faucet_resource: ResourceAddress,
    account: ComponentAddress,
    account_proof: NonFungibleAddress,
    account_key: RistrettoSecretKey,
}

fn setup() -> AssertTest {
    let mut template_test = TemplateTest::new(CRATE_PATH, [TEMPLATE_PATH]);

    // Create user account to receive faucet tokens
    let (account, account_proof, account_key) = template_test.create_funded_account();

    AssertTest {
        template_test,
        faucet_resource: TARI_TOKEN,
        account,
        account_proof,
        account_key,
    }
}

mod assert_bucket_contains {
    use super::*;

    #[test]
    fn it_asserts_that_a_bucket_contains() {
        let mut test: AssertTest = setup();

        test.template_test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(test.account, "withdraw", args![test.faucet_resource, 1000u64])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                .assert_bucket_contains_at_least("faucet_bucket", test.faucet_resource, FAUCET_WITHDRAWAL_AMOUNT)
                .assert_bucket_contains_exactly("faucet_bucket", test.faucet_resource, FAUCET_WITHDRAWAL_AMOUNT)
                .assert_bucket_contains_at_most("faucet_bucket", test.faucet_resource, FAUCET_WITHDRAWAL_AMOUNT)
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );
    }

    #[test]
    fn it_fails_with_invalid_resource() {
        let mut test: AssertTest = setup();

        // we are going to assert a different resource than the faucet resource
        let invalid_resource_address = NFT_FAUCET_RESOURCE_ADDRESS;

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(test.account, "withdraw", args![test.faucet_resource, 1000u64])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                .assert_bucket_contains_at_least("faucet_bucket", invalid_resource_address, FAUCET_WITHDRAWAL_AMOUNT)
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(
            reason,
            RuntimeError::AssertError(AssertError::InvalidResource {
                expected: invalid_resource_address,
                got: test.faucet_resource,
            }),
        );
    }

    #[test]
    fn it_fails_with_invalid_amount() {
        let mut test: AssertTest = setup();

        // we are going to assert that the faucet bucket has more tokens that it really has
        let min_amount = FAUCET_WITHDRAWAL_AMOUNT + 1;

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(test.account, "withdraw", args![test.faucet_resource, 1000u64])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                // Passes
                .assert_bucket_contains_exactly("faucet_bucket", test.faucet_resource, FAUCET_WITHDRAWAL_AMOUNT)
                // Passes
                .assert_bucket_contains_at_most("faucet_bucket", test.faucet_resource, min_amount)
                // Fails
                .assert_bucket_contains_at_least("faucet_bucket", test.faucet_resource, min_amount)
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(
            reason,
            RuntimeError::AssertError(AssertError::BucketAmountAssertionFail {
                expected: min_amount.into(),
                check: CheckOrd::Gte,
                got: FAUCET_WITHDRAWAL_AMOUNT.into(),
            }),
        );
    }

    #[test]
    fn it_fails_with_invalid_bucket() {
        let mut test: AssertTest = setup();

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(test.account, "withdraw", args![test.faucet_resource, 1000u64])
                // we are going to assert a workspace value that is NOT a bucket
                .call_method(test.account, "get_balances", args![])
                .put_last_instruction_output_on_workspace("invalid_bucket")
                .assert_bucket_contains_at_least("invalid_bucket", test.faucet_resource, FAUCET_WITHDRAWAL_AMOUNT)
                .call_method(test.account, "deposit", args![Workspace("invalid_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(
            reason,
            RuntimeError::AssertError(AssertError::NotABucket {
                key: WorkspaceOffsetId::new(0),
            }),
        );
    }

    #[test]
    fn it_fails_with_invalid_workspace_key() {
        let mut test: AssertTest = setup();

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(test.account, "withdraw", args![test.faucet_resource, 1000u64])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                // we are going to assert a key that does not exist in the workspace
                // assert_bucket_contains would panic if called with a non-existing key
                .add_instruction(Instruction::Assert {
                    key: WorkspaceOffsetId::new(999),
                    assertion: Assertion::BucketAmount {
                        resource_address: test.faucet_resource,
                        is: CheckOrd::Gte,
                        amount: FAUCET_WITHDRAWAL_AMOUNT.into()
                    }
                })
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(reason, RuntimeError::ItemNotOnWorkspace {
            id: WorkspaceOffsetId::new(999),
            existing_ids: vec![0],
        });
    }

    /// The ids a reject reason lists are part of the reason validators compare, so they must come out in the order
    /// they went on rather than in a hash order that differs between runs.
    #[test]
    fn it_lists_existing_workspace_ids_in_insertion_order() {
        let mut test: AssertTest = setup();

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(test.account, "withdraw", args![test.faucet_resource, 1u64])
                .put_last_instruction_output_on_workspace("first")
                .call_method(test.account, "withdraw", args![test.faucet_resource, 1u64])
                .put_last_instruction_output_on_workspace("second")
                .call_method(test.account, "withdraw", args![test.faucet_resource, 1u64])
                .put_last_instruction_output_on_workspace("third")
                .add_instruction(Instruction::Assert {
                    key: WorkspaceOffsetId::new(999),
                    assertion: Assertion::IsNotNull,
                })
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(reason, RuntimeError::ItemNotOnWorkspace {
            id: WorkspaceOffsetId::new(999),
            existing_ids: vec![0, 1, 2],
        });
    }
}

mod assert_is_not_null {
    use super::*;

    #[test]
    fn it_fails_with_invalid_workspace_key() {
        let mut test: AssertTest = setup();

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                // we are going to assert a key that does not exist in the workspace
                // assert_bucket_contains would panic if called with a non-existing key
                .add_instruction(Instruction::Assert {
                    key: WorkspaceOffsetId::new(999),
                    assertion: Assertion::IsNotNull
                })
                .build_and_seal(&test.account_key),
            vec![],
        );

        // NOT saying that the assertion itself is invalid, but that the key does not exist in the workspace
        assert_reject_reason(reason, RuntimeError::ItemNotOnWorkspace {
            id: WorkspaceOffsetId::new(999),
            existing_ids: vec![],
        });
    }

    #[test]
    fn it_asserts_that_a_workspace_item_is_not_null() {
        let mut test: AssertTest = setup();

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(test.account, "withdraw", args![test.faucet_resource, 1000u64])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                .assert_workspace_item_is_not_null("faucet_bucket")
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .put_last_instruction_output_on_workspace("should_be_null")
                .assert_workspace_item_is_not_null("should_be_null")
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(reason, AssertError::ValueIsNull);
    }
}

mod assert_bucket_contains_non_fungibles {
    use tari_ootle_transaction::NftCheck;

    use super::*;

    #[test]
    fn it_checks_for_nfts() {
        let mut test: AssertTest = setup();

        test.template_test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(NFT_FAUCET_COMPONENT_ADDRESS, "mint", args![5, tari_bor::Value::Null])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                .assert_bucket_contains_non_fungibles_all("faucet_bucket", NFT_FAUCET_RESOURCE_ADDRESS, vec![
                    NonFungibleId::Uint64(0),
                    NonFungibleId::Uint64(1),
                    NonFungibleId::Uint64(2),
                ])
                .assert_bucket_contains_non_fungibles_all("faucet_bucket", NFT_FAUCET_RESOURCE_ADDRESS, vec![
                    NonFungibleId::Uint64(3),
                    NonFungibleId::Uint64(4),
                ])
                // This essentially asserts that the resource is a particular nft resource
                .assert_bucket_contains_non_fungibles_all("faucet_bucket", NFT_FAUCET_RESOURCE_ADDRESS, vec![])
                .assert_bucket_contains_non_fungibles_any("faucet_bucket", NFT_FAUCET_RESOURCE_ADDRESS, vec![
                    NonFungibleId::Uint64(3),
                    NonFungibleId::Uint64(100),
                ])
                .assert_bucket_contains_non_fungibles_none_of("faucet_bucket", NFT_FAUCET_RESOURCE_ADDRESS, vec![
                    NonFungibleId::Uint64(5),
                    NonFungibleId::Uint64(100),
                ])
                .assert_bucket_contains_non_fungibles_not_any_of("faucet_bucket", NFT_FAUCET_RESOURCE_ADDRESS, vec![
                    NonFungibleId::Uint64(1),
                    NonFungibleId::Uint64(100),
                ])
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );
    }

    #[test]
    fn it_fails_if_fungible_resource_provided() {
        let mut test: AssertTest = setup();

        // Take some tokens from the faucet to get a bucket with non-fungibles
        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(test.account, "withdraw", args![test.faucet_resource, 1000u64])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                .assert_bucket_contains_non_fungibles_all("faucet_bucket", test.faucet_resource, vec![])
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(reason, AssertError::InvalidResourceType {
            expected: ResourceType::NonFungible,
            got: ResourceType::Stealth,
        });
    }

    #[test]
    fn it_fails_if_nft_not_present() {
        let mut test: AssertTest = setup();

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(NFT_FAUCET_COMPONENT_ADDRESS, "mint", args![5, tari_bor::Value::Null])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                .assert_bucket_contains_non_fungibles_all("faucet_bucket", NFT_FAUCET_RESOURCE_ADDRESS, vec![
                    NonFungibleId::Uint64(0),
                    NonFungibleId::Uint64(5),
                ])
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(reason, AssertError::BucketContainsNonFungiblesAssertionFail {
            nft: NonFungibleId::Uint64(5),
            check: NftCheck::AllOf,
        });
    }

    #[test]
    fn it_fails_if_nft_none_present() {
        let mut test: AssertTest = setup();

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(NFT_FAUCET_COMPONENT_ADDRESS, "mint", args![5, tari_bor::Value::Null])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                .assert_bucket_contains_non_fungibles_any("faucet_bucket", NFT_FAUCET_RESOURCE_ADDRESS, vec![
                    NonFungibleId::Uint64(5),
                    NonFungibleId::Uint64(6),
                ])
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(reason, AssertError::BucketContainsNonFungiblesAnyAssertionFail {
            check: NftCheck::AnyOf,
        });
    }

    #[test]
    fn it_fails_if_assertion_none_of_all_fails() {
        let mut test: AssertTest = setup();

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(NFT_FAUCET_COMPONENT_ADDRESS, "mint", args![5, tari_bor::Value::Null])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                .assert_bucket_contains_non_fungibles_none_of("faucet_bucket", NFT_FAUCET_RESOURCE_ADDRESS, vec![
                    NonFungibleId::Uint64(0),
                    NonFungibleId::Uint64(6),
                ])
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(reason, AssertError::BucketContainsNonFungiblesAssertionFail {
            nft: NonFungibleId::Uint64(0),
            check: NftCheck::NoneOf,
        });
    }

    #[test]
    fn it_fails_if_assertion_none_of_any_fails() {
        let mut test: AssertTest = setup();

        let reason = test.template_test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_method(NFT_FAUCET_COMPONENT_ADDRESS, "mint", args![5, tari_bor::Value::Null])
                .put_last_instruction_output_on_workspace("faucet_bucket")
                .assert_bucket_contains_non_fungibles_not_any_of("faucet_bucket", NFT_FAUCET_RESOURCE_ADDRESS, vec![
                    NonFungibleId::Uint64(1),
                    NonFungibleId::Uint64(0),
                ])
                .call_method(test.account, "deposit", args![Workspace("faucet_bucket")])
                .build_and_seal(&test.account_key),
            vec![test.account_proof.clone()],
        );

        assert_reject_reason(reason, AssertError::BucketContainsNonFungiblesAnyAssertionFail {
            check: NftCheck::NotAllOf,
        });
    }
}
