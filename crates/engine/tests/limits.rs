//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::BTreeSet, slice};

use tari_bor::encoded_len;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine::{fees::FeeTable, runtime::LimitError, state_store::StateWriter, wasm::WasmExecutionError};
use tari_engine_types::{
    commit_result::RejectReason,
    events::Event,
    limits,
    resource_container::ResourceContainer,
    substate::{Substate, SubstateId, SubstateValue},
    vault::Vault,
};
use tari_ootle_transaction::{Epoch, Transaction, args, builder::named_args::NamedArg};
use tari_template_abi::CallInfo;
use tari_template_lib::{
    args::{ConsensusAction, ConsensusInvokeArg, InvokeResult},
    types::{
        ComponentAddress,
        Metadata,
        NonFungibleId,
        TemplateAddress,
        bytes::Bytes,
        constants::{NFT_FAUCET_COMPONENT_ADDRESS, NFT_FAUCET_RESOURCE_ADDRESS},
    },
};
use tari_template_test_tooling::{TemplateTest, support::assert_error::assert_reject_reason};

const TEMPLATE_PATHS: &[&str] = &["tests/templates/limits"];
const TEMPLATE_NAME: &str = "PushItToTheLimit";
const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn max_call_size_limit() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let max_bytes = Bytes::from(vec![123u8; limits::ENGINE_LIMITS.max_call_size]);
    let value = tari_bor::to_value(&max_bytes).unwrap();
    let call_size = CallInfo::encode_v1_packed_size(slice::from_ref(&value)).unwrap();
    let overhead = call_size - limits::ENGINE_LIMITS.max_call_size;

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(
                template,
                "new",
                args!(Bytes::from(vec![123u8; limits::ENGINE_LIMITS.max_call_size - overhead])),
            )
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(
                template,
                "new",
                args!(Bytes::from(vec![123u8; limits::ENGINE_LIMITS.max_call_size])),
            )
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, WasmExecutionError::CallSizeLimitExceeded {
        limit: limits::ENGINE_LIMITS.max_call_size,
    });
}

#[test]
fn max_random_bytes_len_limit() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);

    let max_len = limits::ENGINE_LIMITS.max_random_bytes_len as u32;

    let bytes: Vec<u8> = test.call_function(TEMPLATE_NAME, "request_random_bytes", args!(max_len), vec![]);
    assert_eq!(bytes.len(), max_len as usize);

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template, "request_random_bytes", args!(max_len + 1))
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, LimitError::MaxRandomBytesLenExceeded {
        len: (max_len + 1) as usize,
    });
}

/// The event `PushItToTheLimit::emit_event_of_size` builds for a payload of `len` bytes. The engine prefixes the
/// topic with the module name and attaches no substate id, the call being a function rather than a method.
fn event_of_size(template: TemplateAddress, len: usize) -> Event {
    let mut payload = Metadata::new();
    payload.insert("data", &"a".repeat(len));
    Event::custom(None, template, format!("{TEMPLATE_NAME}.big"), payload)
}

/// The largest payload whose whole event still fits within `max_event_size_bytes`. Solved for rather than computed,
/// because a CBOR length prefix widens as the payload crosses 24, 256 and 65536 bytes.
fn largest_fitting_payload(template: TemplateAddress) -> usize {
    let limit = limits::ENGINE_LIMITS.max_event_size_bytes;
    let mut len = limit - encoded_len(&event_of_size(template, 0));
    while encoded_len(&event_of_size(template, len)) > limit {
        len -= 1;
    }
    len
}

#[test]
fn max_event_size_limit() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let max_len = largest_fitting_payload(template);

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template, "emit_event_of_size", args!(max_len as u32))
            .build_and_seal(test.secret_key()),
        vec![],
    );
    assert!(
        result
            .finalize
            .events
            .iter()
            .any(|event| event.topic().ends_with(".big")),
        "the event at the size limit is emitted"
    );

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template, "emit_event_of_size", args!(max_len as u32 + 1))
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, LimitError::EventSizeExceeded {
        size: encoded_len(&event_of_size(template, max_len + 1)),
    });
}

/// A vault holding `count` non-fungible ids of the builtin NFT faucet resource. The ids are seeded far above the
/// faucet's own serial numbers so that a later faucet mint never collides with a seeded id.
fn nft_vault_of(count: u64) -> SubstateValue {
    const SEED_BASE: u64 = 1_000_000;
    let ids = (SEED_BASE..SEED_BASE + count)
        .map(NonFungibleId::Uint64)
        .collect::<BTreeSet<_>>();
    SubstateValue::Vault(Vault::new(ResourceContainer::non_fungible(
        NFT_FAUCET_RESOURCE_ADDRESS,
        ids,
    )))
}

/// The largest non-fungible id count whose vault substate still fits within `max_substate_size`.
fn largest_fitting_nft_count() -> u64 {
    let limit = limits::ENGINE_LIMITS.max_substate_size;
    let (mut lo, mut hi) = (0u64, 200_000u64);
    assert!(encoded_len(&nft_vault_of(hi)) > limit, "the search is bracketed");
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if encoded_len(&nft_vault_of(mid)) <= limit {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// A transaction minting `batches` faucet-sized batches of tokens from the builtin NFT faucet into `account`.
fn deposit_nfts(account: ComponentAddress, key: &RistrettoSecretKey, batches: u64) -> Transaction {
    // The builtin faucet mints fewer than ten tokens per call.
    const PER_MINT: u64 = 9;

    let mut builder = Transaction::builder_localnet(Epoch(1));
    for batch in 0..batches {
        let workspace_key = format!("nft{batch}");
        builder = builder
            .call_method(NFT_FAUCET_COMPONENT_ADDRESS, "mint", args![
                PER_MINT,
                tari_bor::Value::Null
            ])
            .put_last_instruction_output_on_workspace(&workspace_key)
            .call_method(account, "deposit", args![Workspace(workspace_key.as_str())]);
    }
    builder.build_and_seal(key)
}

/// Overwrites `account`'s NFT faucet vault with one holding `count` ids, keeping its substate version.
fn seed_nft_vault(test: &mut TemplateTest, account: ComponentAddress, count: u64) {
    let vault_id = *test
        .read_only_state_store()
        .get_vaults_for_component(account)
        .unwrap()
        .iter()
        .find(|(_, vault)| *vault.resource_address() == NFT_FAUCET_RESOURCE_ADDRESS)
        .expect("the account holds a vault for the NFT faucet resource")
        .0;
    let version = test
        .read_only_state_store()
        .get_substate(&SubstateId::Vault(vault_id))
        .unwrap()
        .version();

    let value = nft_vault_of(count);
    assert!(
        encoded_len(&value) <= limits::ENGINE_LIMITS.max_substate_size,
        "the seeded vault is itself within the limit, so only the deposit can take it over"
    );
    test.get_state_store_mut()
        .set_state(SubstateId::Vault(vault_id), Substate::new(version, value))
        .unwrap();
}

/// The size limit binds on every substate a transaction persists, not only on those it creates: a vault grows one
/// deposit at a time and crosses the limit in a transaction that creates no substate of its own.
#[test]
fn max_substate_size_limit_applies_to_mutations() {
    // Enough tokens per deposit that the vault's growth dwarfs the slack between the largest fitting id count and
    // the limit itself.
    const DEPOSIT_BATCHES: u64 = 5;

    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (account, owner_proof, account_key) = test.create_funded_account();

    // Give the account a vault for the faucet resource to seed.
    test.execute_expect_success(deposit_nfts(account, &account_key, 1), vec![owner_proof.clone()]);

    let max_count = largest_fitting_nft_count();

    // A deposit into a vault with room to spare is accepted.
    seed_nft_vault(&mut test, account, max_count - 100);
    test.execute_expect_success(deposit_nfts(account, &account_key, DEPOSIT_BATCHES), vec![
        owner_proof.clone(),
    ]);

    // One that takes the vault over the limit is not.
    seed_nft_vault(&mut test, account, max_count);
    let reason = test.execute_expect_failure(deposit_nfts(account, &account_key, DEPOSIT_BATCHES), vec![owner_proof]);
    assert_reject_reason(reason, "exceeds the maximum allowed size");
}

/// A transaction of `count` components, each holding 64 KiB of data.
fn create_components(template: TemplateAddress, key: &RistrettoSecretKey, count: usize) -> Transaction {
    let mut builder = Transaction::builder_localnet(Epoch(1));
    for _ in 0..count {
        builder = builder.call_function(template, "new", args!(Bytes::from(vec![7u8; 64 * 1024])));
    }
    builder.build_and_seal(key)
}

#[test]
fn max_transaction_output_bytes_limit() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let per_component = 64 * 1024;
    let fitting = limits::ENGINE_LIMITS.max_transaction_output_bytes / per_component - 4;

    test.execute_expect_success(create_components(template, test.secret_key(), fitting), vec![]);

    let reason = test.execute_expect_failure(create_components(template, test.secret_key(), fitting + 8), vec![]);
    assert_reject_reason(
        reason,
        format!(
            "exceeding the maximum of {} bytes per transaction",
            limits::ENGINE_LIMITS.max_transaction_output_bytes
        ),
    );
}

/// The bytes `PushItToTheLimit::return_items` returns for `len`, and the CBOR items they hold.
fn returned_items(len: u32) -> (u64, u64) {
    let raw = tari_bor::encode(&vec![0u32; len as usize]).unwrap();
    (raw.len() as u64, tari_bor::count_data_items(&raw).unwrap())
}

fn return_value_charge(len: u32) -> u64 {
    let (bytes, items) = returned_items(len);
    limits::return_value_points(bytes, items)
}

fn native_points_for(test: &mut TemplateTest, function: &str, args: Vec<NamedArg>) -> u64 {
    let template = test.get_template_address(TEMPLATE_NAME);
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template, function, args)
            .build_and_seal(test.secret_key()),
        vec![],
    );
    result.native_execution_points
}

/// Decoding and indexing a returned value runs on the host, outside the Wasmer meter, and costs a
/// roughly fixed amount per CBOR item, so the engine charges native points for each item it is
/// handed.
#[test]
fn a_return_value_is_charged_per_item() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (small, large) = (1, 30_000);

    let small_points = native_points_for(&mut test, "return_items", args![small]);
    let large_points = native_points_for(&mut test, "return_items", args![large]);

    assert_eq!(
        large_points - small_points,
        return_value_charge(large) - return_value_charge(small),
        "returning {large} items rather than {small} must cost exactly the difference in return-value charges"
    );
}

/// A value a nested cross-template call returns is handled by the engine on its way back to the
/// caller just as a top-level one is, and is charged the same.
#[test]
fn a_nested_return_value_is_charged_per_item() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let (small, large) = (1, 30_000);

    let small_points = native_points_for(&mut test, "return_items_nested", args![template, small]);
    let large_points = native_points_for(&mut test, "return_items_nested", args![template, large]);

    // The outer call passes `len` to the engine and returns the count, and both encodings widen
    // with it. The engine reads `len` three times: in the engine call's argument, in the call's
    // arguments nested in it, and as the callee's argument nested in those. The callee's value then
    // reaches the caller as the engine call's response.
    let width = |len: u32| tari_bor::encode(&len).unwrap().len() as u64;
    let count_charge = |len: u32| limits::return_value_points(width(len), 1);
    let call_arg_charge = |len: u32| 3 * limits::engine_arg_points(width(len), 0);
    let response_charge = |len: u32| {
        let (bytes, items) = returned_items(len);
        limits::engine_response_points(bytes, items)
    };
    assert_eq!(
        large_points - small_points,
        return_value_charge(large) + count_charge(large) + call_arg_charge(large) + response_charge(large) -
            return_value_charge(small) -
            count_charge(small) -
            call_arg_charge(small) -
            response_charge(small),
        "a nested call returning {large} items rather than {small} must cost exactly the difference in return-value \
         charges"
    );
}

/// A return value priced above what the transaction can cover fails it out of compute, with the
/// return-value charge as the points it required. Run in the fee intent, whose flat credit is the
/// allowance.
#[test]
fn an_unaffordable_return_value_fails_out_of_compute() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let mut fee_table = FeeTable::zero_rated();
    fee_table.per_wasm_point_cost = 1;
    fee_table.wasm_points_cost_divisor = 1;
    test.set_fee_table(fee_table);
    test.enable_fees();

    let mut len =
        u32::try_from(limits::FREE_COMPUTE_GRACE_POINTS / limits::NativeExecutionPoints::PER_RETURN_VALUE_ITEM)
            .unwrap();
    while return_value_charge(len) <= limits::FREE_COMPUTE_GRACE_POINTS {
        len += 1;
    }
    assert!(
        returned_items(len).0 <= limits::ENGINE_LIMITS.max_call_size as u64,
        "a return value priced above the fee intent's credit must fit within max_call_size for this test to reach the \
         charge"
    );

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| builder.call_function(template, "return_items", args![len]))
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(
        reason,
        format!("requiring {} points exceeds the fee intent's", return_value_charge(len)),
    );
}

/// Native points a main-instruction call to `engine_call_with_items` is charged. The engine refuses
/// the argument, so the transaction fails, and the charge stands all the same.
fn engine_call_native_points(test: &mut TemplateTest, items: u32) -> u64 {
    let template = test.get_template_address(TEMPLATE_NAME);
    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .call_function(template, "engine_call_with_items", args![items])
                .build_and_seal(test.secret_key()),
            vec![],
        )
        .unwrap();
    assert_reject_reason(
        result.expect_failure().clone(),
        "Failed to decode argument for engine call",
    );
    result.native_execution_points
}

/// The bytes of the argument `engine_call_with_items` sends for `items`, and the CBOR items they
/// hold: a five-byte array head and one byte per element.
fn engine_arg_shape(items: u32) -> (u64, u64) {
    (5 + u64::from(items), 1 + u64::from(items))
}

/// Walking and decoding an engine call's argument runs on the host, outside the Wasmer meter, so
/// the engine charges native points for each byte and item it reads, whether or not the argument
/// turns out to be one the op accepts.
#[test]
fn an_engine_call_argument_is_charged_per_byte_and_item() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (small, large) = (1, 30_000);

    let small_points = engine_call_native_points(&mut test, small);
    let large_points = engine_call_native_points(&mut test, large);

    let charge = |items| {
        let (bytes, items) = engine_arg_shape(items);
        limits::engine_arg_points(bytes, items)
    };
    assert_eq!(
        large_points - small_points,
        charge(large) - charge(small),
        "an argument of {large} items rather than {small} must cost exactly the difference in engine argument charges"
    );
}

/// Runs `engine_call_with_items` for `items` in the fee intent, whose flat credit is the allowance,
/// and returns why it failed.
fn engine_call_in_fee_intent(items: u32) -> RejectReason {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let mut fee_table = FeeTable::zero_rated();
    fee_table.per_wasm_point_cost = 1;
    fee_table.wasm_points_cost_divisor = 1;
    test.set_fee_table(fee_table);
    test.enable_fees();

    assert!(
        engine_arg_shape(items).0 <= limits::ENGINE_LIMITS.max_internal_call_size as u64,
        "the argument must fit within max_internal_call_size for this test to reach the charge"
    );
    test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| {
                builder.call_function(template, "engine_call_with_items", args![items])
            })
            .build_and_seal(test.secret_key()),
        vec![],
    )
}

/// An argument holding more items than the transaction can pay for fails it out of compute. The
/// engine reads no more of it than the transaction can pay for, and is paid for what it read: the
/// credit is spent to within an item's price.
#[test]
fn an_unaffordable_engine_call_argument_fails_out_of_compute_once_paid_for_what_was_read() {
    let per_item = limits::NativeExecutionPoints::PER_ENGINE_ARG_ITEM;
    let items = u32::try_from(limits::FREE_COMPUTE_GRACE_POINTS / per_item + 1_000).unwrap();

    let reason = engine_call_in_fee_intent(items);
    assert_reject_reason(
        reason.clone(),
        format!(
            "requiring {} points exceeds the fee intent's",
            limits::engine_arg_points(0, 1)
        ),
    );
    let message = reason.to_string();
    let consumed: u64 = message
        .split("compute credit: ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|consumed| consumed.parse().ok())
        .unwrap_or_else(|| panic!("no consumed points in {message}"));
    assert!(
        consumed > limits::FREE_COMPUTE_GRACE_POINTS - per_item,
        "the items read must be paid for: {consumed} of {} consumed",
        limits::FREE_COMPUTE_GRACE_POINTS
    );
}

/// `max_internal_call_size` sits at what a template can build in its linear memory, so building an
/// argument that large runs a template out of memory, trapping before the engine sees it, where one
/// a quarter that size reaches the engine.
#[test]
fn a_template_cannot_build_an_engine_call_argument_at_the_size_limit() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let mut log_of_size = |len: usize| {
        test.execute_expect_failure(
            Transaction::builder_localnet(Epoch(1))
                .call_function(template, "engine_log_of_size", args![u32::try_from(len).unwrap()])
                .build_and_seal(test.secret_key()),
            vec![],
        )
    };

    let limit = limits::ENGINE_LIMITS.max_internal_call_size;
    assert_reject_reason(log_of_size(limit / 4), "Log entry of");
    assert_reject_reason(log_of_size(limit), "Wasm RuntimeError: unreachable");
}

/// Native points a transaction calling `method` on a new `PushItToTheLimit` component is charged.
fn native_points_for_method(test: &mut TemplateTest, method: &str, args: Vec<NamedArg>) -> u64 {
    let component: ComponentAddress =
        test.call_function(TEMPLATE_NAME, "new", args![Bytes::from(vec![0u8; 4])], vec![]);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(component, method, args)
            .build_and_seal(test.secret_key()),
        vec![],
    )
    .native_execution_points
}

/// A component's state reaches the engine as a byte string inside an engine call's argument, one
/// item there, and is decoded again as a value. That decode is charged for the items the state
/// holds.
#[test]
fn a_component_state_is_charged_per_item_it_holds() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (small, large) = (1u32, 30_000u32);

    let small_points = native_points_for_method(&mut test, "set_items", args![small]);
    let large_points = native_points_for_method(&mut test, "set_items", args![large]);

    let items = u64::from(large - small);
    assert!(
        large_points - small_points >= limits::engine_arg_points(0, items),
        "a state holding {items} more items must be charged for each: {} < {}",
        large_points - small_points,
        limits::engine_arg_points(0, items)
    );
}

/// A cross-template call's arguments reach the engine as byte strings nested two deep in an engine
/// call's argument, and are decoded again into the callee's call. That decode is charged for the
/// items they hold.
#[test]
fn a_cross_template_call_argument_is_charged_per_item_it_holds() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let (small, large) = (1u32, 30_000u32);

    let small_points = native_points_for(&mut test, "pass_items_nested", args![template, small]);
    let large_points = native_points_for(&mut test, "pass_items_nested", args![template, large]);

    let items = u64::from(large - small);
    assert!(
        large_points - small_points >= limits::engine_arg_points(0, items),
        "an argument holding {items} more items must be charged for each: {} < {}",
        large_points - small_points,
        limits::engine_arg_points(0, items)
    );
}

/// A `PushItToTheLimit` component whose state holds `data_len` bytes of data and `items` zeros.
fn component_with_state(test: &mut TemplateTest, data_len: usize, items: u32) -> ComponentAddress {
    let component: ComponentAddress =
        test.call_function(TEMPLATE_NAME, "new", args![Bytes::from(vec![0u8; data_len])], vec![]);
    if items > 0 {
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component, "set_items", args![items])
                .build_and_seal(test.secret_key()),
            vec![],
        );
    }
    component
}

/// Native points `reads` more reads of `component`'s state, each answered with the state and left
/// undecoded, are charged than a single read.
fn native_points_for_extra_reads(test: &mut TemplateTest, component: ComponentAddress, reads: u32) -> u64 {
    let mut native_points = |reads: u32| {
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_method(component, "read_state_undecoded", args![reads])
                .build_and_seal(test.secret_key()),
            vec![],
        )
        .native_execution_points
    };
    native_points(1 + reads) - native_points(1)
}

/// An engine call's response is built and copied on the host, outside the Wasmer meter, and the
/// template pays no more metered WASM for a large response than a small one. So each read of a
/// component's state is charged for each byte and item the state holds, whether or not the template
/// decodes it.
#[test]
fn an_engine_call_response_is_charged_per_byte_and_item() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (small_data, large_data) = (4, 64 * 1024);
    let large_items = 10_000u32;
    let small = component_with_state(&mut test, small_data, 0);
    let large = component_with_state(&mut test, large_data, large_items);
    let reads = 10;

    let small_points = native_points_for_extra_reads(&mut test, small, reads);
    let large_points = native_points_for_extra_reads(&mut test, large, reads);

    let extra_bytes = encoded_len(&Bytes::from(vec![0u8; large_data])) + encoded_len(&vec![0u32; large_items as usize]) -
        encoded_len(&Bytes::from(vec![0u8; small_data])) -
        encoded_len(&Vec::<u32>::new());
    assert_eq!(
        large_points - small_points,
        u64::from(reads) * limits::engine_response_points(extra_bytes as u64, u64::from(large_items)),
        "{reads} reads of a state larger by {extra_bytes} bytes and {large_items} items must cost exactly the \
         difference in engine response charges"
    );
}

/// Every engine call is charged a flat cost for the host work around it, on top of its argument and
/// its response, whether or not the template reads the response.
#[test]
fn an_engine_call_is_charged_a_flat_cost() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let mut native_points = |count: u32| {
        test.execute_expect_success(
            Transaction::builder_localnet(Epoch(1))
                .call_function(template, "read_epoch_undecoded", args![count])
                .build_and_seal(test.secret_key()),
            vec![],
        )
        .native_execution_points
    };
    let calls = 100u32;
    let extra_points = native_points(1 + calls) - native_points(1);

    let arg = tari_bor::encode(&ConsensusInvokeArg {
        action: ConsensusAction::GetCurrentEpoch,
    })
    .unwrap();
    let response = tari_bor::encode(&InvokeResult::encode(&Epoch(0)).unwrap()).unwrap();
    let per_call = limits::NativeExecutionPoints::PER_ENGINE_CALL +
        limits::engine_arg_points(arg.len() as u64, tari_bor::count_data_items(&arg).unwrap()) +
        limits::engine_response_points(response.len() as u64, tari_bor::count_data_items(&response).unwrap());
    assert_eq!(
        extra_points,
        u64::from(calls) * per_call,
        "{calls} engine calls must each be charged the flat cost, their argument and their response"
    );
}

/// Engine call responses are charged against the transaction's compute allowance, so a template
/// that reads a large state over and over fails out of compute. Run in the fee intent, whose flat
/// credit is the allowance, with too few reads for their metered WASM alone to exhaust it.
#[test]
fn engine_call_responses_are_charged_against_the_fee_intent_credit() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let data_len = 64 * 1024;
    let component = component_with_state(&mut test, data_len, 0);
    let mut fee_table = FeeTable::zero_rated();
    fee_table.per_wasm_point_cost = 1;
    fee_table.wasm_points_cost_divisor = 1;
    test.set_fee_table(fee_table);
    test.enable_fees();

    let reads = limits::FREE_COMPUTE_GRACE_POINTS / limits::engine_response_points(data_len as u64, 1) + 1;
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| {
                builder.call_method(component, "read_state_undecoded", args![u32::try_from(reads).unwrap()])
            })
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert_reject_reason(reason, "exceeds the fee intent's");
}
