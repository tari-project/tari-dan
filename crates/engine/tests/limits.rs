//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::BTreeSet, slice};

use tari_bor::encoded_len;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine::{runtime::LimitError, state_store::StateWriter, wasm::WasmExecutionError};
use tari_engine_types::{
    events::Event,
    limits,
    resource_container::ResourceContainer,
    substate::{Substate, SubstateId, SubstateValue},
    vault::Vault,
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_abi::CallInfo;
use tari_template_lib::types::{
    ComponentAddress,
    Metadata,
    NonFungibleId,
    TemplateAddress,
    bytes::Bytes,
    constants::{NFT_FAUCET_COMPONENT_ADDRESS, NFT_FAUCET_RESOURCE_ADDRESS},
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
    payload.insert("data", "a".repeat(len));
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

/// Overwrites the account's NFT faucet vault with one holding `count` ids, keeping its substate version.
fn seed_nft_vault(test: &mut TemplateTest, count: u64) {
    let (vault_id, version) = test
        .get_state_store_mut()
        .iter()
        .find_map(|(id, substate)| {
            let SubstateId::Vault(vault_id) = id else {
                return None;
            };
            let vault = substate.substate_value().vault()?;
            (*vault.resource_address() == NFT_FAUCET_RESOURCE_ADDRESS).then_some((*vault_id, substate.version()))
        })
        .expect("the account holds a vault for the NFT faucet resource");

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
    seed_nft_vault(&mut test, max_count - 100);
    test.execute_expect_success(deposit_nfts(account, &account_key, DEPOSIT_BATCHES), vec![
        owner_proof.clone(),
    ]);

    // One that takes the vault over the limit is not.
    seed_nft_vault(&mut test, max_count);
    let reason = test.execute_expect_failure(deposit_nfts(account, &account_key, DEPOSIT_BATCHES), vec![owner_proof]);
    assert_reject_reason(reason, "exceeds the maximum allowed size");
}
