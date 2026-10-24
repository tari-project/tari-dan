//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Golden-vector **generator** + **runner**.
//!
//! Two entry points, both `#[test]`s in this file:
//!
//! - **Generator** ([`regen_fixtures`]) — gated on `OOTLE_REGEN_FIXTURES=1`. It (re)seeds the sample vector if absent,
//!   then re-runs the core over every fixture's `input` and rewrites `expected` + `provenance`. **The generator is the
//!   only writer of `expected`** — humans never hand-edit it.
//!
//!   ```text
//!   OOTLE_REGEN_FIXTURES=1 cargo test -p ootle_sdk_core --test golden_vectors regen_fixtures
//!   ```
//!
//! - **Runner** ([`run_golden_vectors`]) — always on. Loads every fixture, runs the named operation over its `input`,
//!   and asserts the core reproduces `expected` **byte-for-byte** by comparing the lowercase-hex strings (never parsed
//!   structures — that would hide CBOR drift). A stale committed `expected` is a red build, not a silent pass.
//!
//! The round-trip / idempotency guard ([`regen_is_idempotent`]) proves that regenerating an existing
//! fixture yields byte-identical JSON (with `git_rev` pinned for hermeticity), so a committed fixture
//! and a freshly generated one agree — the staleness contract.

mod harness;

use std::{fs, path::PathBuf};

use harness::{
    Fixture,
    OP_ACCOUNT_BALANCES,
    OP_BUILD_AND_ENCODE_FAUCET_CLAIM,
    OP_BUILD_AND_ENCODE_INSTRUCTIONS,
    OP_BUILD_AND_ENCODE_PUBLIC_TRANSFER,
    OP_BUILD_AND_ENCODE_STEALTH_TRANSFER,
    OP_BUILD_STEALTH_OUTPUTS_STATEMENT,
    OP_COSIGN_SEAL_WITH_AUTH,
    OP_DECODE_STEALTH_UTXO,
    OP_DECODE_SUBSTATE,
    OP_DERIVE_ACCOUNT_ADDRESS,
    OP_DERIVE_ACCOUNT_KEY_FROM_SEED,
    OP_DERIVE_VIEW_KEY_FROM_SEED,
    OP_ENCODE_ARG,
    OP_FORMAT_IDENTITY_ADDRESS,
    OP_PARSE_ADDRESS,
    OP_PARSE_FINALIZED_RESULT,
    OP_RESOLVE_AND_ENCODE_PUBLIC_TRANSFER,
    OP_SCAN_STEALTH_OUTPUT,
    Provenance,
    SCHEMA_VERSION,
    StealthScanInput,
    VectorInput,
    VectorKeys,
    VectorStealthKeys,
    current_provenance,
    fixture_to_pretty_json,
    fixtures_dir,
    load_all_fixtures,
    provenance_with_rev,
    regenerate,
    regenerate_with_provenance,
    run_operation,
};
use ootle_sdk_core::{
    ArgValue,
    BlobSpec,
    ComponentRef,
    FaucetClaimIntent,
    FeeSource,
    FetchedSubstate,
    GenericTransactionIntent,
    InstructionSpec,
    types::{
        address::{ComponentAddressStr, ResourceAddressStr},
        bytes::{BuildSeed, PublicKeyBytes, SecretKeyBytes},
        intent::{InputRef, PublicTransferIntent, TransferRecipient},
        network::Network,
        numeric::BoundaryAmount,
    },
};
use tari_crypto::{
    keys::PublicKey as _,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_engine_types::{
    component::{Component, ComponentBody, ComponentHeader},
    encoded::Encoded,
    resource_container::ResourceContainer,
    substate::{SubstateId, SubstateValue},
    vault::Vault,
};
use tari_ootle_common_types::SubstateVersion;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib_types::{
    Amount,
    ClaimedOutputTombstoneAddress,
    ComponentAddress,
    EntityId,
    Hash32,
    NonFungibleAddress,
    NonFungibleId,
    ObjectKey,
    ResourceAddress,
    SubstateOwnerRule,
    TransactionReceiptAddress,
    UtxoAddress,
    UtxoId,
    ValidatorFeePoolAddress,
    VaultId,
    access_rules::ComponentAccessRules,
    crypto::RistrettoPublicKeyBytes,
};

/// The committed sample vector's relative path (a trivially-valid, obviously-a-sample fixture). The
/// *real*, cross-validated public-transfer vectors live alongside it (see [`REAL_VECTORS`]).
const SAMPLE_REL_PATH: &str = "public_transfer/sample_single_key_basic.json";

/// One golden-vector case: a label (the fixture file's relative path) and the seed builder that produces its
/// [`Fixture`]. Aliased so the `&[VectorCase]` tables below read cleanly.
type VectorCase = (&'static str, fn() -> Fixture);

/// The first **real** (non-sample), cross-validated public-transfer vectors. Each is seeded
/// from its `input` builder if the file is missing; the generator owns `expected`. The runner
/// (`run_golden_vectors`) and the `ootle-rs` orchestration-parity cross-check
/// (`tests/ootle_rs_crosscheck.rs`) both read these committed files — never hand-edit `expected`.
///
/// - `single_key_basic` — a plain single-key public transfer (modest amount).
/// - `large_amount` — same scenario with an amount `> 2^53` µTari, proving the u64-safe path carries it intact
///   end-to-end.
const REAL_VECTORS: &[VectorCase] = &[
    ("public_transfer/single_key_basic.json", single_key_basic_fixture_seed),
    ("public_transfer/large_amount.json", large_amount_fixture_seed),
];

/// The **resolved-path** vectors: `resolve_and_encode_public_transfer` over an intent with
/// **no** explicit inputs, plus a fabricated fetched batch that resolves the 3-item want set. These
/// lock the resolved seal/encode bytes — incl. the canonical resolved-input ordering and the u64
/// boundary surviving resolution (`resolve_large_amount`).
const RESOLVE_VECTORS: &[VectorCase] = &[
    (
        "resolve_public_transfer/single_key_basic.json",
        resolve_single_key_basic_fixture_seed,
    ),
    (
        "resolve_public_transfer/large_amount.json",
        resolve_large_amount_fixture_seed,
    ),
];

/// A fixed, canonical Ristretto scalar from a small seed (low scalars are always canonical — never
/// `0xff..ff`, which is not canonical and would be a `KEY` error).
fn fixed_scalar_bytes(seed: u8) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[0] = seed;
    // Assert canonical so a bad seed fails loudly at authoring time rather than silently.
    RistrettoSecretKey::from_canonical_bytes(&b).expect("low scalar is canonical");
    b
}

/// Authors the sample vector's `input` (the `expected` is filled by the generator, never by hand).
fn sample_input() -> VectorInput {
    let component = ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH])).to_string();
    let resource = ResourceAddress::new(ObjectKey::from_array([0xbb; ObjectKey::LENGTH])).to_string();

    let intent = PublicTransferIntent {
        from_account: ComponentAddressStr::parse(&component).expect("valid component"),
        recipient: TransferRecipient::PublicKey(PublicKeyBytes::from_array([5u8; 32])),
        resource_address: ResourceAddressStr::parse(&resource).expect("valid resource"),
        // An amount well above 2^53 µTari proves the u64-boundary path carries it intact.
        amount: BoundaryAmount::new((1u64 << 53) + 12_345),
        fee: BoundaryAmount::new(2000),
        inputs: vec![InputRef::versioned(component, 0)],
        min_epoch: Some(5),
        max_epoch: 99,
        dry_run: false,
    };

    let keys = VectorKeys {
        account_secret: SecretKeyBytes::from_array(fixed_scalar_bytes(11)),
    };

    VectorInput {
        network: Some(Network::Esmeralda),
        intent: Some(intent),
        keys: Some(keys),
        fetched: None,
        ..Default::default()
    }
}

/// Builds the sample fixture shell (name/op/input + placeholder expected). `expected`/`provenance`
/// are overwritten by [`regenerate`] before anything is written.
fn sample_fixture_seed() -> Fixture {
    Fixture {
        name: "sample/public_transfer_single_key_basic".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: "semantic".to_string(),
        provenance: current_provenance(),
        operation: OP_BUILD_AND_ENCODE_PUBLIC_TRANSFER.to_string(),
        input: sample_input(),
        // Filled by the generator — never trusted as committed.
        expected: harness::ExpectedOutput::default(),
    }
}

// --- Real (non-sample) public-transfer vectors ----------------------------------------
//
// These author only the deterministic `input`; the generator fills `expected` (never hand-edited).
// The fixed key/nonce material reuses the same canonical low-scalar convention as the sample, with
// distinct seeds per vector so each vector's signatures are independent.

/// Authors the `single_key_basic` real vector's `input`: a plain single-key public transfer of a
/// modest µTari amount, with one explicit input, fixed epochs, and pinned auth/seal nonces.
fn single_key_basic_input() -> VectorInput {
    let component = ComponentAddress::new(ObjectKey::from_array([0x11; ObjectKey::LENGTH])).to_string();
    let resource = ResourceAddress::new(ObjectKey::from_array([0x22; ObjectKey::LENGTH])).to_string();

    let intent = PublicTransferIntent {
        from_account: ComponentAddressStr::parse(&component).expect("valid component"),
        recipient: TransferRecipient::PublicKey(PublicKeyBytes::from_array([7u8; 32])),
        resource_address: ResourceAddressStr::parse(&resource).expect("valid resource"),
        amount: BoundaryAmount::new(1_000_000),
        fee: BoundaryAmount::new(2500),
        inputs: vec![InputRef::versioned(component, 0)],
        min_epoch: Some(1),
        max_epoch: 10,
        dry_run: false,
    };

    let keys = VectorKeys {
        account_secret: SecretKeyBytes::from_array(fixed_scalar_bytes(101)),
    };

    VectorInput {
        network: Some(Network::Esmeralda),
        intent: Some(intent),
        keys: Some(keys),
        fetched: None,
        ..Default::default()
    }
}

/// The `single_key_basic` fixture shell (placeholder `expected`, overwritten by the generator).
fn single_key_basic_fixture_seed() -> Fixture {
    Fixture {
        name: "public_transfer/single_key_basic".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: "semantic".to_string(),
        provenance: current_provenance(),
        operation: OP_BUILD_AND_ENCODE_PUBLIC_TRANSFER.to_string(),
        input: single_key_basic_input(),
        expected: harness::ExpectedOutput::default(),
    }
}

/// Authors the `large_amount` real vector's `input`: the same single-key shape, but with an amount
/// `> 2^53` µTari so the runner + cross-check exercise the u64-safe path end-to-end.
fn large_amount_input() -> VectorInput {
    let mut input = single_key_basic_input();
    // (1 << 53) + 987_654 — comfortably above the f64 integer-safe ceiling (2^53).
    input.intent.as_mut().expect("encode input has an intent").amount = BoundaryAmount::new((1u64 << 53) + 987_654);
    input
}

/// The `large_amount` fixture shell (placeholder `expected`, overwritten by the generator).
fn large_amount_fixture_seed() -> Fixture {
    Fixture {
        name: "public_transfer/large_amount".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: "semantic".to_string(),
        provenance: current_provenance(),
        operation: OP_BUILD_AND_ENCODE_PUBLIC_TRANSFER.to_string(),
        input: large_amount_input(),
        expected: harness::ExpectedOutput::default(),
    }
}

// --- Resolved-path vectors ------------------------------------------------------------
//
// These author the deterministic `input` for `resolve_and_encode_public_transfer`: an intent with NO
// explicit inputs (so the resolved path runs) plus a fabricated `fetched` batch that satisfies the
// 3-item public-transfer want set. The generator fills `expected`. The fixed addresses/keys reuse the
// canonical low-scalar convention with distinct seeds.

/// The from-account component the resolved vectors pay from.
fn resolve_from_component() -> ComponentAddress {
    ComponentAddress::new(ObjectKey::from_array([0x31; ObjectKey::LENGTH]))
}

/// The resource transferred (non-TARI, so it is added as an explicit input on resolution).
fn resolve_resource() -> ResourceAddress {
    ResourceAddress::new(ObjectKey::from_array([0x32; ObjectKey::LENGTH]))
}

/// The from-vault (holding the resource) referenced by the from-component's state.
fn resolve_from_vault_id() -> VaultId {
    VaultId::new(ObjectKey::from_array([0x33; ObjectKey::LENGTH]))
}

/// The recipient public key (canonical low scalar) the transfer is sent to.
fn resolve_recipient_pk() -> RistrettoPublicKeyBytes {
    let pk = RistrettoPublicKey::from_secret_key(
        &RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(34)).expect("low scalar is canonical"),
    );
    RistrettoPublicKeyBytes::from_bytes(pk.as_bytes()).expect("32-byte pk")
}

/// A `SubstateValue::Component` JSON referencing `vault_ids`, exactly as the indexer hands it back.
fn resolve_component_json(vault_ids: &[VaultId]) -> serde_json::Value {
    let state = tari_bor::to_value(&vault_ids.to_vec()).expect("encode vault ids");
    let component = Component {
        header: ComponentHeader {
            template_address: ACCOUNT_TEMPLATE_ADDRESS,
            owner_rule: SubstateOwnerRule::None,
            access_rules: ComponentAccessRules::new(),
            entity_id: EntityId::from_array([0u8; EntityId::LENGTH]),
        },
        body: ComponentBody::from_cbor_value(state),
    };
    indexer_substate_json(&SubstateValue::Component(component))
}

/// A `SubstateValue::Vault` JSON holding `resolve_resource()`.
fn resolve_vault_json() -> serde_json::Value {
    let vault = Vault::new(ResourceContainer::public_fungible(
        resolve_resource(),
        Amount::new(1_000_000),
    ));
    indexer_substate_json(&SubstateValue::Vault(vault))
}

/// The single-round fetched batch that resolves the want set: the from-component (revealing its
/// vault) + the from-vault (holding the resource). The optional recipient component/vault are omitted
/// (`create_account_with_bucket` creates them), so this one batch resolves everything.
fn resolve_fetched_batch() -> Vec<FetchedSubstate> {
    vec![
        FetchedSubstate {
            substate_id: SubstateId::Component(resolve_from_component()).to_string(),
            version: 0,
            substate_value: resolve_component_json(&[resolve_from_vault_id()]),
        },
        FetchedSubstate {
            substate_id: SubstateId::Vault(resolve_from_vault_id()).to_string(),
            version: 0,
            substate_value: resolve_vault_json(),
        },
    ]
}

/// Authors the `resolve_public_transfer/single_key_basic` `input`: an intent with **no** explicit
/// inputs (the resolved path) plus the fetched batch, with pinned auth/seal nonces.
fn resolve_single_key_basic_input() -> VectorInput {
    let intent = PublicTransferIntent {
        from_account: ComponentAddressStr::from_internal(&resolve_from_component()),
        recipient: TransferRecipient::PublicKey(PublicKeyBytes::from_bytes(resolve_recipient_pk().as_bytes()).unwrap()),
        resource_address: ResourceAddressStr::from_internal(&resolve_resource()),
        amount: BoundaryAmount::new(1_000_000),
        fee: BoundaryAmount::new(2500),
        // No explicit inputs ⇒ the resolution path runs.
        inputs: vec![],
        min_epoch: Some(1),
        max_epoch: 10,
        dry_run: false,
    };

    let keys = VectorKeys {
        account_secret: SecretKeyBytes::from_array(fixed_scalar_bytes(111)),
    };

    VectorInput {
        network: Some(Network::Esmeralda),
        intent: Some(intent),
        keys: Some(keys),
        fetched: Some(resolve_fetched_batch()),
        ..Default::default()
    }
}

/// The `resolve_public_transfer/single_key_basic` fixture shell (generator fills `expected`).
fn resolve_single_key_basic_fixture_seed() -> Fixture {
    Fixture {
        name: "resolve_public_transfer/single_key_basic".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: "semantic".to_string(),
        provenance: current_provenance(),
        operation: OP_RESOLVE_AND_ENCODE_PUBLIC_TRANSFER.to_string(),
        input: resolve_single_key_basic_input(),
        expected: harness::ExpectedOutput::default(),
    }
}

/// Authors the `resolve_public_transfer/large_amount` `input`: same resolved shape, amount `> 2^53`
/// µTari — proving the u64 boundary survives resolution + seal, not just the explicit path.
fn resolve_large_amount_input() -> VectorInput {
    let mut input = resolve_single_key_basic_input();
    input.intent.as_mut().expect("encode input has an intent").amount = BoundaryAmount::new((1u64 << 53) + 987_654);
    input
}

/// The `resolve_public_transfer/large_amount` fixture shell (generator fills `expected`).
fn resolve_large_amount_fixture_seed() -> Fixture {
    Fixture {
        name: "resolve_public_transfer/large_amount".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: "semantic".to_string(),
        provenance: current_provenance(),
        operation: OP_RESOLVE_AND_ENCODE_PUBLIC_TRANSFER.to_string(),
        input: resolve_large_amount_input(),
        expected: harness::ExpectedOutput::default(),
    }
}

// --- Result-parse vectors -------------------------------------------------------------
//
// These author only the `input.raw_result` — a raw INDEXER finalized-result JSON, exactly as
// `GET /transactions/{id}/result` returns it. The generator fills `expected.parsed` by running
// the core parser. The runner compares the parsed structure (canonicalized JSON), not raw bytes —
// there is no CBOR byte stream here. Three cases lock the three TransactionResult arms, including the
// `EpochExpired` abort drift case.

/// The committed result-parse vectors: a full Accept (fees + event + diff), an AcceptFeeRejectRest, a
/// Reject whose reason is `Abort{EpochExpired}` — the canonical-AbortReason drift case — and a Reject
/// whose reason is a classified `ExecutionFailure`, which pins the failure sub-code the same way.
const PARSE_VECTORS: &[VectorCase] = &[
    ("parse_finalized_result/accept.json", parse_accept_fixture_seed),
    (
        "parse_finalized_result/accept_fee_reject_rest.json",
        parse_accept_fee_reject_rest_fixture_seed,
    ),
    (
        "parse_finalized_result/reject_epoch_expired.json",
        parse_reject_epoch_expired_fixture_seed,
    ),
    (
        "parse_finalized_result/reject_access_denied.json",
        parse_reject_access_denied_fixture_seed,
    ),
    ("parse_finalized_result/dry_run.json", parse_dry_run_fixture_seed),
];

/// A `SubstateDiff` with one created component (`up`) and one destroyed vault (`down`).
fn parse_accept_diff() -> tari_engine_types::substate::SubstateDiff {
    use tari_engine_types::substate::{Substate, SubstateDiff};
    let mut diff = SubstateDiff::new();
    let component = ComponentAddress::new(ObjectKey::from_array([0x41; ObjectKey::LENGTH]));
    let vault_id = VaultId::new(ObjectKey::from_array([0x42; ObjectKey::LENGTH]));
    diff.up(
        SubstateId::Component(component),
        Substate::new(
            SubstateVersion::ZERO,
            SubstateValue::Component(parse_sample_component(&[VaultId::new(ObjectKey::from_array(
                [0x43; ObjectKey::LENGTH],
            ))])),
        ),
    );
    diff.down(SubstateId::Vault(vault_id), SubstateVersion::new(1));
    diff
}

/// A minimal account `Component` referencing `vault_ids` (reuses the resolve-vector builder pattern).
fn parse_sample_component(vault_ids: &[VaultId]) -> Component {
    Component {
        header: ComponentHeader {
            template_address: ACCOUNT_TEMPLATE_ADDRESS,
            owner_rule: SubstateOwnerRule::None,
            access_rules: ComponentAccessRules::new(),
            entity_id: EntityId::from_array([0u8; EntityId::LENGTH]),
        },
        body: ComponentBody::from_cbor_value(tari_bor::to_value(&vault_ids.to_vec()).expect("encode vault ids")),
    }
}

/// A fee receipt with a `> 2^53` amount, so the parse vector proves the u64-safe path survives parsing.
fn parse_fee_receipt() -> tari_engine_types::fees::FeeReceipt {
    use tari_engine_types::fees::{FeeBreakdown, FeeReceiptBuilder, FeeSource};
    let mut breakdown = FeeBreakdown::default();
    breakdown.add(FeeSource::Initial, 1000);
    breakdown.add(FeeSource::Storage, (1u64 << 53) + 7);
    FeeReceiptBuilder::default()
        .with_total_fee_payment((1u64 << 53) + 2000)
        .with_total_fees_paid((1u64 << 53) + 1500)
        .with_total_fee_overcharge(0)
        .with_cost_breakdown(breakdown)
        .build()
}

/// Builds an engine `ExecuteResult` for the given `TransactionResult`, with a fixed fee receipt, one
/// event, and one log — the realistic payload a finalized transaction carries.
fn parse_execute_result(
    result: tari_engine_types::commit_result::TransactionResult,
    epoch: Option<u64>,
) -> tari_engine_types::commit_result::ExecuteResult {
    use tari_engine_types::{
        commit_result::{ExecuteResult, FinalizeResult},
        events::Event,
        logs::LogEntry,
    };
    use tari_template_lib_types::{Hash32, LogLevel, Metadata};

    let mut payload = Metadata::new();
    payload.insert("amount", "1000000");
    let event = Event::new(
        Some(SubstateId::Component(ComponentAddress::new(ObjectKey::from_array(
            [0x41; ObjectKey::LENGTH],
        )))),
        ACCOUNT_TEMPLATE_ADDRESS,
        "std.deposit".to_string(),
        payload,
    );

    let finalize = FinalizeResult {
        transaction_hash: Hash32::from_array([0x51; 32]),
        events: vec![event],
        logs: vec![LogEntry::new(LogLevel::Info, "transfer executed".to_string())],
        result,
        total_fees_required: parse_fee_receipt().total_fees_charged(),
        fee_receipt: parse_fee_receipt(),
    };

    ExecuteResult {
        finalize,
        execution_time: std::time::Duration::from_secs(1),
        execute_epoch: epoch.map(tari_engine_types::Epoch),
        wasm_execution_points: 0,
        native_execution_points: 0,
    }
}

/// `value` as the indexer serves it: its CBOR encoding, base64.
fn indexer_substate_json(value: &SubstateValue) -> serde_json::Value {
    serde_json::to_value(Encoded::encode(value).expect("substate value encodes")).expect("base64 serializes")
}

/// Wraps an `ExecuteResult` in the indexer `Finalized` envelope JSON (exactly the indexer REST shape).
fn parse_finalized_wire_json(result: tari_engine_types::commit_result::ExecuteResult) -> serde_json::Value {
    use tari_consensus_types::Decision;
    let decision = Decision::from(&result.finalize.result);
    serde_json::json!({
        "Finalized": {
            "final_decision": decision,
            "execution_result": Encoded::encode(&result).expect("execution result encodes"),
            "execution_time": { "secs": 1, "nanos": 0 },
            "finalized_time": "2026-06-12 00:00:00.0",
            "abort_details": serde_json::Value::Null,
        }
    })
}

/// Builds a parse-vector fixture shell for `name` over a raw indexer JSON `raw`. The generator fills
/// `expected.parsed`.
fn parse_fixture_seed(name: &str, raw: serde_json::Value) -> Fixture {
    Fixture {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: OP_PARSE_FINALIZED_RESULT.to_string(),
        input: VectorInput {
            raw_result: Some(raw),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// Accept vector: a full commit with fees, an event, and a diff.
fn parse_accept_fixture_seed() -> Fixture {
    use tari_engine_types::commit_result::TransactionResult;
    let raw = parse_finalized_wire_json(parse_execute_result(
        TransactionResult::Accept(parse_accept_diff()),
        Some(42),
    ));
    parse_fixture_seed("parse_finalized_result/accept", raw)
}

/// AcceptFeeRejectRest vector: the fee intent committed but the main intent rejected.
fn parse_accept_fee_reject_rest_fixture_seed() -> Fixture {
    use tari_engine_types::commit_result::{RejectReason, TransactionResult};
    let raw = parse_finalized_wire_json(parse_execute_result(
        TransactionResult::AcceptFeeRejectRest(
            parse_accept_diff(),
            RejectReason::execution_failure_unclassified("main intent failed"),
        ),
        Some(7),
    ));
    parse_fixture_seed("parse_finalized_result/accept_fee_reject_rest", raw)
}

/// Reject vector whose reason is `Abort{EpochExpired}` — the canonical-AbortReason drift case. Proves
/// the parser surfaces `EPOCH_EXPIRED` as a stable abort sub-code.
fn parse_reject_epoch_expired_fixture_seed() -> Fixture {
    use tari_engine_types::commit_result::{AbortReason, RejectReason, TransactionResult};
    let raw = parse_finalized_wire_json(parse_execute_result(
        TransactionResult::Reject(RejectReason::Abort {
            reason: AbortReason::EpochExpired,
        }),
        None,
    ));
    parse_fixture_seed("parse_finalized_result/reject_epoch_expired", raw)
}

/// Reject vector whose reason is an `ExecutionFailure` carrying a classified code. Proves the parser
/// surfaces `ACCESS_DENIED` as a stable failure sub-code rather than leaving it inside the message.
fn parse_reject_access_denied_fixture_seed() -> Fixture {
    use tari_engine_types::commit_result::{ExecutionFailureCode, RejectReason, TransactionResult};
    let raw = parse_finalized_wire_json(parse_execute_result(
        TransactionResult::Reject(RejectReason::ExecutionFailure {
            code: ExecutionFailureCode::AccessDenied,
            message: "At instruction #0: Access Denied: Withdraw".to_string(),
        }),
        None,
    ));
    parse_fixture_seed("parse_finalized_result/reject_access_denied", raw)
}

/// Wraps an `ExecuteResult` in the indexer **dry-run** response JSON
/// (`SubmitTransactionDryRunResponse { transaction_id, result }`, `result` its base64 CBOR encoding) — the
/// `POST /transactions/dry-run` shape. The parser shape-dispatches on the top-level `result` key and fills
/// `estimated_fee`.
fn parse_dry_run_wire_json(result: tari_engine_types::commit_result::ExecuteResult) -> serde_json::Value {
    serde_json::json!({
        "transaction_id": result.finalize.transaction_hash,
        "result": Encoded::encode(&result).unwrap(),
    })
}

/// Dry-run vector: an executed-but-uncommitted transaction whose `estimated_fee` is surfaced
/// (`required_fees`, the metered cost plus the dry-run fee allowance). Proves the additive field rides the existing
/// parse op, the `> 2^53` fee stays a bare u64, and the committed fixtures (which omit `estimated_fee`) are
/// unaffected.
fn parse_dry_run_fixture_seed() -> Fixture {
    use tari_engine_types::commit_result::TransactionResult;
    let raw = parse_dry_run_wire_json(parse_execute_result(
        TransactionResult::Accept(parse_accept_diff()),
        Some(11),
    ));
    parse_fixture_seed("parse_finalized_result/dry_run", raw)
}

// --- Stealth outputs-statement vectors ------------------------------------------------
//
// These author the pinned `StealthTransferIntent` + `StealthEntropy`; the generator fills
// `expected.stealth_outputs_statement` (the statement with the byte-unstable `agg_range_proof`
// nulled) + `expected.aggregated_output_mask`. The comparison mode is **semantic**:
// the runner re-validates the freshly built statement cryptographically and compares the
// deterministic fields. Two cases: one plain stealth output (no view key) and one with a resource
// view key (exercising the injected ElGamal/ZK nonces + the viewable-balance proof).

/// The committed stealth outputs-statement vectors. Each authors only the deterministic `input`; the
/// generator owns `expected`.
const STEALTH_OUTPUTS_VECTORS: &[VectorCase] = &[
    (
        "stealth_outputs_statement/single_output_no_view_key.json",
        stealth_single_output_seed,
    ),
    (
        "stealth_outputs_statement/single_output_with_view_key.json",
        stealth_view_key_output_seed,
    ),
];

/// A fixed canonical Ristretto public key from a small secret seed.
fn stealth_pk(seed: u8) -> PublicKeyBytes {
    let sk = RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(seed)).expect("canonical");
    let pk = RistrettoPublicKey::from_secret_key(&sk);
    PublicKeyBytes::from_bytes(pk.as_bytes()).expect("32-byte pk")
}

/// The TARI stealth resource address as a boundary string.
fn stealth_resource() -> ResourceAddressStr {
    ResourceAddressStr::parse(tari_template_lib_types::constants::STEALTH_TARI_RESOURCE_ADDRESS.to_string())
        .expect("valid resource")
}

/// A fixed build seed (per-fixture-distinct via `byte`) the stealth ops expand into proof entropy.
fn stealth_build_seed(byte: u8) -> BuildSeed {
    BuildSeed::from_array([byte; 32])
}

/// Builds a single-output stealth intent (optionally with a resource view key).
fn stealth_intent(amount: u64, with_view: bool) -> ootle_sdk_core::types::stealth::StealthTransferIntent {
    use ootle_sdk_core::types::stealth::{StealthOutputSpec, StealthPayTo};
    ootle_sdk_core::types::stealth::StealthTransferIntent {
        from_account: ComponentAddressStr::parse(
            ComponentAddress::new(ObjectKey::from_array([0x5a; ObjectKey::LENGTH])).to_string(),
        )
        .expect("valid component"),
        resource_address: stealth_resource(),
        fee: BoundaryAmount::new(2000),
        inputs: vec![],
        outputs: vec![StealthOutputSpec {
            destination_account_pk: stealth_pk(60),
            destination_view_pk: stealth_pk(61),
            amount,
            revealed_amount: 0,
            resource_address: stealth_resource(),
            resource_view_key: with_view.then(|| stealth_pk(62)),
            memo: None,
            pay_to: StealthPayTo::StealthPublicKey,
            utxo_tag: None,
            minimum_value_promise: 0,
        }],
        revealed_input_amount: 0,
        revealed_output_amount: 0,
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
        pay_fee_from_revealed: false,
    }
}

/// Builds a stealth fixture shell (semantic compare; the generator fills `expected`).
fn stealth_fixture_seed(
    name: &str,
    intent: ootle_sdk_core::types::stealth::StealthTransferIntent,
    seed: BuildSeed,
) -> Fixture {
    Fixture {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        compare: "semantic".to_string(),
        provenance: current_provenance(),
        operation: OP_BUILD_STEALTH_OUTPUTS_STATEMENT.to_string(),
        input: VectorInput {
            network: Some(Network::Esmeralda),
            stealth_intent: Some(intent),
            stealth_seed: Some(seed),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// Vector 1: a single stealth output, no resource view key.
fn stealth_single_output_seed() -> Fixture {
    stealth_fixture_seed(
        "stealth_outputs_statement/single_output_no_view_key",
        stealth_intent(1_000_000, false),
        stealth_build_seed(70),
    )
}

/// Vector 2: a single stealth output with a resource view key (viewable-balance proof present).
fn stealth_view_key_output_seed() -> Fixture {
    stealth_fixture_seed(
        "stealth_outputs_statement/single_output_with_view_key",
        stealth_intent(2_500_000, true),
        stealth_build_seed(80),
    )
}

// --- Full stealth-send vectors --------------------------------------------------------
//
// These author the pinned `StealthTransferIntent` + `StealthEntropy` + `StealthKeys` (+ a fabricated
// fetched UTXO for the stealth-input case); the generator fills `expected.sealed_transaction_semantic`
// (the decoded sealed tx with the byte-unstable proofs/signature scalars nulled). The comparison mode
// is **semantic**: the runner re-seals, re-validates every signature, and compares the
// deterministic decoded fields.
//
// Two of the three seal cases are reachable through the validating full pipeline and are vectored
// here: the **stealth `c+k` seal** (a fabricated stealth-UTXO input) and the **account-key seal**
// (a revealed-input bucket). The **ephemeral seal** is NOT reachable via the full pipeline:
// it requires `must_sign_with_account_key == false` (⇒ no revealed input) AND no stealth inputs, but the
// engine's `validate_transfer` pre-flight rejects a statement with neither inputs nor revealed inputs.
// The ephemeral seal path is therefore covered by the unit tests in `stealth/sign_seal.rs` (which inject
// an ephemeral-shaped partial directly), not by a full-send golden vector.

/// The committed full stealth-send vectors. Each authors only the deterministic `input`; the
/// generator owns `expected`.
const STEALTH_TRANSFER_VECTORS: &[VectorCase] = &[
    (
        "stealth_transfer/stealth_seal_with_input.json",
        stealth_send_stealth_seal_seed,
    ),
    (
        "stealth_transfer/account_key_seal_with_revealed_input.json",
        stealth_send_account_key_seed,
    ),
    (
        "stealth_transfer/revealed_output_single.json",
        stealth_send_revealed_output_single_seed,
    ),
    (
        "stealth_transfer/revealed_output_multi.json",
        stealth_send_revealed_output_multi_seed,
    ),
];

/// The pinned stealth seal-key bundle the send vectors use (canonical low scalars).
fn stealth_send_keys(seed_byte: u8) -> VectorStealthKeys {
    VectorStealthKeys {
        account_secret: SecretKeyBytes::from_array(fixed_scalar_bytes(11)),
        // The view secret the send vectors' stealth input is encrypted to.
        view_secret: SecretKeyBytes::from_array(fixed_scalar_bytes(162)),
        // The build seed expands into both the account-key nonces and the proof entropy; a distinct
        // byte per fixture keeps the vectors independent.
        seed: BuildSeed::from_array([seed_byte; 32]),
    }
}

/// The send-vector from-account component.
fn stealth_send_from_component() -> ComponentAddressStr {
    ComponentAddressStr::parse(ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH])).to_string())
        .expect("valid component")
}

/// One plain stealth output (no view key) for the send vectors.
fn stealth_send_output(amount: u64) -> ootle_sdk_core::types::stealth::StealthOutputSpec {
    stealth_send_output_full(3, 4, amount, 0)
}

/// One stealth output (no view key) for the send vectors, with explicit destination key seeds, a
/// blinded `amount`, and a per-output `revealed_amount` deposit slice.
fn stealth_send_output_full(
    account_pk_seed: u8,
    view_pk_seed: u8,
    amount: u64,
    revealed_amount: u64,
) -> ootle_sdk_core::types::stealth::StealthOutputSpec {
    use ootle_sdk_core::types::stealth::{StealthOutputSpec, StealthPayTo};
    StealthOutputSpec {
        destination_account_pk: stealth_pk(account_pk_seed),
        destination_view_pk: stealth_pk(view_pk_seed),
        amount,
        revealed_amount,
        resource_address: stealth_resource(),
        resource_view_key: None,
        memo: None,
        pay_to: StealthPayTo::StealthPublicKey,
        utxo_tag: None,
        minimum_value_promise: 0,
    }
}

/// Builds a full stealth-send fixture shell (semantic compare; the generator fills `expected`).
fn stealth_send_fixture_seed(name: &str, input: VectorInput) -> Fixture {
    Fixture {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        compare: "semantic".to_string(),
        provenance: current_provenance(),
        operation: OP_BUILD_AND_ENCODE_STEALTH_TRANSFER.to_string(),
        input,
        expected: harness::ExpectedOutput::default(),
    }
}

/// Vector 1 — stealth `c+k` seal: one fabricated, decryptable stealth UTXO input balanced
/// against an equal-value stealth output. The decrypted input is promoted to the seal signer, so the
/// transaction is sealed with its one-time `c+k` key.
fn stealth_send_stealth_seal_seed() -> Fixture {
    use ootle_byte_type::ToByteType;
    use ootle_sdk_core::{
        FetchedSubstate,
        stealth::inputs::stealth_utxo_substate_id,
        types::stealth::{CommitmentBytes, StealthInputSpec, StealthTransferIntent},
    };
    use tari_engine_types::{
        Utxo,
        UtxoOutput,
        crypto::{OutputBody, commit_u64_amount},
        substate::SubstateValue,
    };
    use tari_ootle_wallet_crypto::{encrypted_data::encrypt_data, kdfs, stealth::condition_root};
    use tari_template_lib_types::{
        access_rules::AccessRule,
        crypto::UtxoTag,
        stealth::{SpendAuthorization, SpendCondition},
    };

    let value = 1_000_000u64;
    let input_mask = RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(160)).expect("canonical");
    let nonce_secret = RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(161)).expect("canonical");
    let public_nonce = RistrettoPublicKey::from_secret_key(&nonce_secret);
    let view_secret = RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(162)).expect("canonical");

    let commitment = commit_u64_amount(&input_mask, value).to_byte_type();
    let commitment_hex = hex::encode(commitment.as_bytes());

    let encryption_key = kdfs::encrypted_data_dh_kdf_aead(&view_secret, &public_nonce);
    let encrypted_data = encrypt_data(value, &input_mask, &encryption_key, None).expect("encrypt");
    let utxo = Utxo::new(UtxoOutput {
        output: OutputBody {
            public_nonce: public_nonce.to_byte_type(),
            encrypted_data,
            minimum_value_promise: 0,
            viewable_balance: None,
        },
        auth: SpendAuthorization::Script(condition_root(&[SpendCondition::access_rule(AccessRule::AllowAll)]).unwrap()),
        tag: UtxoTag::new(0),
    });

    // The input's owner account is the seal-key account (so c+k derives from `stealth_send_keys`).
    let owner_secret = RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(11)).expect("canonical");
    let owner_pk = RistrettoPublicKey::from_secret_key(&owner_secret);
    let owner_pk_bytes = PublicKeyBytes::from_bytes(owner_pk.as_bytes()).expect("32-byte pk");

    let substate_id = stealth_utxo_substate_id(stealth_resource().as_str(), &commitment_hex).expect("id");
    let fetched = vec![FetchedSubstate {
        substate_id: substate_id.to_string(),
        version: 0,
        substate_value: indexer_substate_json(&SubstateValue::Utxo(utxo)),
    }];

    let intent = StealthTransferIntent {
        from_account: stealth_send_from_component(),
        resource_address: stealth_resource(),
        fee: BoundaryAmount::new(2000),
        inputs: vec![StealthInputSpec {
            commitment: CommitmentBytes::from_hex(&commitment_hex).expect("commitment hex"),
            owner_account_pk: owner_pk_bytes,
        }],
        outputs: vec![stealth_send_output(value)],
        revealed_input_amount: 0,
        revealed_output_amount: 0,
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
        pay_fee_from_revealed: false,
    };

    let input = VectorInput {
        network: Some(Network::Esmeralda),
        stealth_intent: Some(intent),
        stealth_keys: Some(stealth_send_keys(90)),
        fetched: Some(fetched),
        spend_secrets: vec![SecretKeyBytes::from_bytes(view_secret.as_bytes()).expect("32-byte secret")],
        ..Default::default()
    };
    stealth_send_fixture_seed("stealth_transfer/stealth_seal_with_input", input)
}

/// Vector 2 — account-key seal: a stealth output funded by a revealed-input bucket
/// (`revealed_input_amount > 0` ⇒ the account key must seal + authorize).
fn stealth_send_account_key_seed() -> Fixture {
    use ootle_sdk_core::types::stealth::StealthTransferIntent;
    let intent = StealthTransferIntent {
        from_account: stealth_send_from_component(),
        resource_address: stealth_resource(),
        fee: BoundaryAmount::new(2000),
        inputs: vec![],
        outputs: vec![stealth_send_output(1_000_000)],
        revealed_input_amount: 1_000_000,
        revealed_output_amount: 0,
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
        pay_fee_from_revealed: false,
    };
    let input = VectorInput {
        network: Some(Network::Esmeralda),
        stealth_intent: Some(intent),
        stealth_keys: Some(stealth_send_keys(110)),
        ..Default::default()
    };
    stealth_send_fixture_seed("stealth_transfer/account_key_seal_with_revealed_input", input)
}

/// Vector 3 — single revealed-output deposit: a revealed-input bucket funds one stealth
/// output of blinded value 1_000_000 plus a 500_000 revealed deposit into the recipient account.
/// Balance: revealed_input 1_500_000 == stealth_out 1_000_000 + revealed_out 500_000. The deposit
/// fold emits a single `create_account_with_bucket("output_bucket")` (no split). `revealed_input > 0`
/// ⇒ account-key seal (reuses `stealth_send_keys`).
fn stealth_send_revealed_output_single_seed() -> Fixture {
    use ootle_sdk_core::types::stealth::StealthTransferIntent;
    let intent = StealthTransferIntent {
        from_account: stealth_send_from_component(),
        resource_address: stealth_resource(),
        fee: BoundaryAmount::new(2000),
        inputs: vec![],
        outputs: vec![stealth_send_output_full(3, 4, 1_000_000, 500_000)],
        revealed_input_amount: 1_500_000,
        revealed_output_amount: 500_000,
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
        pay_fee_from_revealed: false,
    };
    let input = VectorInput {
        network: Some(Network::Esmeralda),
        stealth_intent: Some(intent),
        stealth_keys: Some(stealth_send_keys(130)),
        ..Default::default()
    };
    stealth_send_fixture_seed("stealth_transfer/revealed_output_single", input)
}

/// Vector 4 — multi revealed-output split: a revealed-input bucket funds two stealth
/// outputs (blinded 500_000 each) plus revealed deposits of 1_000_000 + 500_000 into two distinct
/// recipient accounts. Balance: revealed_input 2_500_000 == stealth_out 1_000_000 + revealed_out
/// 1_500_000. The deposit fold emits a `take_from_bucket` + `create_account_with_bucket` per output
/// (`output-sub-bucket-{i}`). `revealed_input > 0` ⇒ account-key seal.
fn stealth_send_revealed_output_multi_seed() -> Fixture {
    use ootle_sdk_core::types::stealth::StealthTransferIntent;
    let intent = StealthTransferIntent {
        from_account: stealth_send_from_component(),
        resource_address: stealth_resource(),
        fee: BoundaryAmount::new(2000),
        inputs: vec![],
        outputs: vec![
            stealth_send_output_full(3, 4, 500_000, 1_000_000),
            stealth_send_output_full(5, 6, 500_000, 500_000),
        ],
        revealed_input_amount: 2_500_000,
        revealed_output_amount: 1_500_000,
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
        pay_fee_from_revealed: false,
    };
    let input = VectorInput {
        network: Some(Network::Esmeralda),
        stealth_intent: Some(intent),
        stealth_keys: Some(stealth_send_keys(140)),
        ..Default::default()
    };
    stealth_send_fixture_seed("stealth_transfer/revealed_output_multi", input)
}

// --- Stealth receive / scan vectors ---------------------------------------------------
//
// These author an inbound stealth UTXO (built FROM the send-side crypto with fixed keys/entropy —
// a round-trip: encrypt → scan decrypts) plus the scan keys; the generator fills `expected.decrypted`
// (the recovered `DecryptedOutput` for a mine UTXO, or JSON `null` for a not-mine UTXO). The
// comparison mode is the default **bytes** — decryption is RNG-free, so the scan output is byte-stable.

/// The committed stealth-scan vectors. Each authors only the deterministic `input`; the generator
/// owns `expected`.
const STEALTH_SCAN_VECTORS: &[VectorCase] = &[
    ("stealth_scan/mine_basic.json", stealth_scan_mine_basic_seed),
    ("stealth_scan/not_mine.json", stealth_scan_not_mine_seed),
    (
        "stealth_scan/foreign_spend_key.json",
        stealth_scan_foreign_spend_key_seed,
    ),
];

/// Builds an inbound stealth UTXO addressed to (`view_pk`, `account_pk`) for `amount`, using the
/// send-side crypto with fixed scalars (a true round-trip the scanner inverts).
fn stealth_scan_inbound(
    network: Network,
    account_pk: &RistrettoPublicKey,
    view_pk: &RistrettoPublicKey,
    nonce_secret: &RistrettoSecretKey,
    mask: &RistrettoSecretKey,
    amount: u64,
) -> ootle_sdk_core::types::stealth::InboundStealthOutput {
    use ootle_byte_type::ToByteType;
    use ootle_sdk_core::types::stealth::{
        CommitmentBytes,
        EncryptedDataBytes,
        InboundStealthOutput,
        StealthPayTo,
        UtxoTagBytes,
    };
    use tari_engine_types::crypto::commit_u64_amount;
    use tari_ootle_wallet_crypto::{StealthCryptoApi, encrypted_data::encrypt_data, kdfs};

    let internal_network: ootle_network::Network = network.into();
    let public_nonce = RistrettoPublicKey::from_secret_key(nonce_secret);
    let crypto = StealthCryptoApi::new();

    // Sender-side AEAD key: (nonce_secret, view_public_key).
    let encryption_key = kdfs::encrypted_data_dh_kdf_aead(nonce_secret, view_pk);
    let encrypted_data = encrypt_data(amount, mask, &encryption_key, None).expect("encrypt");
    let commitment = commit_u64_amount(mask, amount).to_byte_type();

    let resource = stealth_resource();
    let resource_internal = resource.to_internal().unwrap();
    let owner_pk = crypto.derive_stealth_owner_public_key(internal_network, account_pk, view_pk, nonce_secret);
    let owner_pk_bytes = PublicKeyBytes::from_bytes(owner_pk.as_bytes()).expect("32-byte pk");
    let tag = crypto.derive_stealth_output_tag(internal_network, nonce_secret, view_pk, &resource_internal);

    InboundStealthOutput {
        commitment: CommitmentBytes::from_bytes(commitment.as_bytes()).expect("32-byte commitment"),
        encrypted_data: EncryptedDataBytes::from_bytes(encrypted_data.as_bytes()),
        sender_public_nonce: PublicKeyBytes::from_bytes(public_nonce.as_bytes()).expect("32-byte pk"),
        pay_to: StealthPayTo::StealthPublicKey,
        spend_public_key: Some(owner_pk_bytes),
        utxo_tag: Some(UtxoTagBytes::from_u32(tag.value())),
        resource_address: resource,
    }
}

/// Builds a stealth-scan fixture shell (default byte compare; the generator fills `expected`).
fn stealth_scan_fixture_seed(name: &str, scan: StealthScanInput) -> Fixture {
    Fixture {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: OP_SCAN_STEALTH_OUTPUT.to_string(),
        input: VectorInput {
            stealth_scan_input: Some(scan),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// The fixed keys/entropy the scan vectors share (canonical low scalars).
fn stealth_scan_keys() -> (
    RistrettoSecretKey,
    RistrettoSecretKey,
    RistrettoSecretKey,
    RistrettoSecretKey,
) {
    // (view_secret, account_secret, nonce_secret, mask)
    (
        RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(120)).expect("canonical"),
        RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(121)).expect("canonical"),
        RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(122)).expect("canonical"),
        RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(123)).expect("canonical"),
    )
}

/// Vector 1 — `mine_basic`: a UTXO encrypted to the scanner's keys; scan recovers value + mask + tag.
fn stealth_scan_mine_basic_seed() -> Fixture {
    let net = Network::Esmeralda;
    let (view_secret, account_secret, nonce_secret, mask) = stealth_scan_keys();
    let view_pk = RistrettoPublicKey::from_secret_key(&view_secret);
    let account_pk = RistrettoPublicKey::from_secret_key(&account_secret);
    let amount = 1_234_567u64;
    let output = stealth_scan_inbound(net, &account_pk, &view_pk, &nonce_secret, &mask, amount);
    let scan = StealthScanInput {
        network: net,
        view_secret: SecretKeyBytes::from_bytes(view_secret.as_bytes()).expect("32-byte secret"),
        account_public_key: PublicKeyBytes::from_bytes(account_pk.as_bytes()).expect("32-byte pk"),
        output,
        skip_memo: true,
    };
    stealth_scan_fixture_seed("stealth_scan/mine_basic", scan)
}

/// Vector 2 — `not_mine`: the same UTXO scanned with a *different* view secret ⇒ `Ok(None)`.
fn stealth_scan_not_mine_seed() -> Fixture {
    let net = Network::Esmeralda;
    let (view_secret, account_secret, nonce_secret, mask) = stealth_scan_keys();
    let view_pk = RistrettoPublicKey::from_secret_key(&view_secret);
    let account_pk = RistrettoPublicKey::from_secret_key(&account_secret);
    let output = stealth_scan_inbound(net, &account_pk, &view_pk, &nonce_secret, &mask, 1_234_567);
    // A different view secret ⇒ wrong AEAD key ⇒ not mine.
    let other_view = RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(200)).expect("canonical");
    let scan = StealthScanInput {
        network: net,
        view_secret: SecretKeyBytes::from_bytes(other_view.as_bytes()).expect("32-byte secret"),
        account_public_key: PublicKeyBytes::from_bytes(account_pk.as_bytes()).expect("32-byte pk"),
        output,
        skip_memo: true,
    };
    stealth_scan_fixture_seed("stealth_scan/not_mine", scan)
}

/// Vector 3 — `foreign_spend_key`: a UTXO that decrypts under the scanner's view key and carries the
/// right tag, but whose spend key pays another account ⇒ `Ok(None)`. A sender can build this to make
/// a view-only wallet credit funds it cannot spend.
fn stealth_scan_foreign_spend_key_seed() -> Fixture {
    let net = Network::Esmeralda;
    let (view_secret, account_secret, nonce_secret, mask) = stealth_scan_keys();
    let view_pk = RistrettoPublicKey::from_secret_key(&view_secret);
    let account_pk = RistrettoPublicKey::from_secret_key(&account_secret);
    let foreign_account_pk = RistrettoPublicKey::from_secret_key(
        &RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(201)).expect("canonical"),
    );
    let output = stealth_scan_inbound(net, &foreign_account_pk, &view_pk, &nonce_secret, &mask, 1_234_567);
    let scan = StealthScanInput {
        network: net,
        view_secret: SecretKeyBytes::from_bytes(view_secret.as_bytes()).expect("32-byte secret"),
        account_public_key: PublicKeyBytes::from_bytes(account_pk.as_bytes()).expect("32-byte pk"),
        output,
        skip_memo: true,
    };
    stealth_scan_fixture_seed("stealth_scan/foreign_spend_key", scan)
}

// --- Stealth UTXO decode vectors ------------------------------------------------------
//
// The decode vector sits AHEAD of the scan group: it authors a fabricated UTXO **substate** (id +
// value, the shape the indexer returns) built from the same fixed send-side crypto, and the
// generator fills `expected.inbound_output` with the decoded `InboundStealthOutput`. Comparison is
// the default **bytes** — the decode is a pure parse + field map (no RNG), so the produced inbound
// output is byte-stable.

/// The committed stealth-decode vectors. Each authors only the deterministic `input` (substate id +
/// value); the generator owns `expected.inbound_output`.
const STEALTH_DECODE_VECTORS: &[VectorCase] = &[("stealth_scan/decode_utxo.json", stealth_decode_utxo_seed)];

/// Vector — `decode_utxo`: a fabricated `StealthPublicKey` UTXO substate (id + value) decodes into
/// the receive-shaped `InboundStealthOutput` the scanner consumes. Built from the same fixed scan
/// keys/entropy as the `mine_basic` scan vector, so the two compose (decode → scan recovers value).
fn stealth_decode_utxo_seed() -> Fixture {
    use ootle_byte_type::ToByteType;
    use tari_engine_types::{
        Utxo,
        UtxoOutput,
        crypto::{OutputBody, commit_u64_amount},
        substate::SubstateValue,
    };
    use tari_ootle_wallet_crypto::{StealthCryptoApi, encrypted_data::encrypt_data, kdfs};
    use tari_template_lib_types::{crypto::UtxoTag, stealth::SpendAuthorization};

    let net = Network::Esmeralda;
    let internal_network: ootle_network::Network = net.into();
    let (view_secret, account_secret, nonce_secret, mask) = stealth_scan_keys();
    let view_pk = RistrettoPublicKey::from_secret_key(&view_secret);
    let account_pk = RistrettoPublicKey::from_secret_key(&account_secret);
    let public_nonce = RistrettoPublicKey::from_secret_key(&nonce_secret);
    let amount = 1_234_567u64;
    let crypto = StealthCryptoApi::new();

    let resource = stealth_resource();
    let resource_internal = resource.to_internal().unwrap();

    // Send-side crypto: AEAD ciphertext, commitment, one-time spend key, scanning tag.
    let encryption_key = kdfs::encrypted_data_dh_kdf_aead(&nonce_secret, &view_pk);
    let encrypted_data = encrypt_data(amount, &mask, &encryption_key, None).expect("encrypt");
    let commitment = commit_u64_amount(&mask, amount).to_byte_type();
    let commitment_hex = hex::encode(commitment.as_bytes());
    let owner_pk = crypto.derive_stealth_owner_public_key(internal_network, &account_pk, &view_pk, &nonce_secret);
    let tag = crypto.derive_stealth_output_tag(internal_network, &nonce_secret, &view_pk, &resource_internal);

    let output_body = OutputBody {
        public_nonce: public_nonce.to_byte_type(),
        encrypted_data,
        minimum_value_promise: 0,
        viewable_balance: None,
    };
    let utxo = Utxo::new(UtxoOutput {
        output: output_body,
        auth: SpendAuthorization::Key(owner_pk.to_byte_type()),
        tag: UtxoTag::new(tag.value()),
    });

    let substate_id = ootle_sdk_core::stealth::stealth_utxo_substate_id(resource.as_str(), &commitment_hex)
        .expect("utxo substate id")
        .to_string();
    let substate_value = indexer_substate_json(&SubstateValue::Utxo(utxo));

    Fixture {
        name: "stealth_scan/decode_utxo".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: OP_DECODE_STEALTH_UTXO.to_string(),
        input: VectorInput {
            substate_id: Some(substate_id),
            substate_value: Some(substate_value),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

// --- Substate decode + account-balance vectors ----------------------------------------
//
// `decode_substate` turns any fetched SubstateValue into the kind-tagged DecodedSubstate;
// `account_balances` sums an account's revealed vault balances per resource. Both are pure parse/sum
// (no RNG), so the default **bytes** compare applies. The account-balances vector pins a balance
// > 2^33 to guard against any float truncation across the boundary. The fixtures are authored from
// the same engine types the indexer serializes, so they cross-check against a LocalNet account-vault
// read.

/// The committed substate-decode vectors. Each authors only the deterministic `input.substate_value`;
/// the generator owns `expected.decoded_substate`.
const SUBSTATE_DECODE_VECTORS: &[VectorCase] = &[
    ("substate_decode/component.json", substate_decode_component_seed),
    (
        "substate_decode/fungible_vault.json",
        substate_decode_fungible_vault_seed,
    ),
];

/// The committed account-balances vectors. Each authors the account component + its vault
/// `FetchedSubstate`s; the generator owns `expected.account_balances`.
const ACCOUNT_BALANCES_VECTORS: &[VectorCase] = &[(
    "account_balances/multi_vault_u64.json",
    account_balances_multi_vault_seed,
)];

/// A fixed vault id for the balance fixtures (lowercase-hex, arbitrary but stable).
fn balance_vault_id(seed: u8) -> VaultId {
    VaultId::new(ObjectKey::from_array([seed; ObjectKey::LENGTH]))
}

/// A fixed resource for the balance fixtures.
fn balance_resource(seed: u8) -> ResourceAddress {
    ResourceAddress::new(ObjectKey::from_array([seed; ObjectKey::LENGTH]))
}

/// An account component whose CBOR state references `vault_ids`, JSON-encoded as the indexer hands it
/// back.
fn balance_account_substate(vault_ids: &[VaultId]) -> serde_json::Value {
    let state = tari_bor::to_value(&vault_ids.to_vec()).expect("encode vault refs");
    let component = Component {
        header: ComponentHeader {
            template_address: ACCOUNT_TEMPLATE_ADDRESS,
            owner_rule: SubstateOwnerRule::None,
            access_rules: ComponentAccessRules::new(),
            entity_id: EntityId::from_array([0u8; EntityId::LENGTH]),
        },
        body: ComponentBody::from_cbor_value(state),
    };
    indexer_substate_json(&SubstateValue::Component(component))
}

/// A fungible vault substate JSON holding `amount` revealed for `resource`.
fn balance_vault_substate(resource: ResourceAddress, amount: u128) -> serde_json::Value {
    let vault = Vault::new(ResourceContainer::public_fungible(resource, Amount::new(amount)));
    indexer_substate_json(&SubstateValue::Vault(vault))
}

/// Vector — `decode_substate/component`: an account component decodes to a kind-tagged Component with
/// its embedded vault ids surfaced.
fn substate_decode_component_seed() -> Fixture {
    let ids = [balance_vault_id(0x11), balance_vault_id(0x12)];
    Fixture {
        name: "substate_decode/component".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: OP_DECODE_SUBSTATE.to_string(),
        input: VectorInput {
            substate_value: Some(balance_account_substate(&ids)),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// Vector — `decode_substate/fungible_vault`: a fungible vault decodes to a kind-tagged Vault with its
/// revealed balance (a value > 2^33) and a zero confidential-commitment count.
fn substate_decode_fungible_vault_seed() -> Fixture {
    let big: u128 = (1u128 << 33) + 987_654_321;
    Fixture {
        name: "substate_decode/fungible_vault".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: OP_DECODE_SUBSTATE.to_string(),
        input: VectorInput {
            substate_value: Some(balance_vault_substate(balance_resource(0x22), big)),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// Vector — `account_balances/multi_vault_u64`: two vaults of the SAME resource sum into one revealed
/// balance > 2^33, proving the u64 path carries the value intact across the boundary.
fn account_balances_multi_vault_seed() -> Fixture {
    let r = balance_resource(0x33);
    let ids = [balance_vault_id(0x41), balance_vault_id(0x42)];
    // Two amounts that sum to > 2^33.
    let a1: u128 = 1u128 << 33;
    let a2: u128 = 123_456_789;
    let vaults = vec![
        FetchedSubstate {
            substate_id: SubstateId::Vault(ids[0]).to_string(),
            version: 0,
            substate_value: balance_vault_substate(r, a1),
        },
        FetchedSubstate {
            substate_id: SubstateId::Vault(ids[1]).to_string(),
            version: 0,
            substate_value: balance_vault_substate(r, a2),
        },
    ];
    Fixture {
        name: "account_balances/multi_vault_u64".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: OP_ACCOUNT_BALANCES.to_string(),
        input: VectorInput {
            substate_value: Some(balance_account_substate(&ids)),
            vault_substates: Some(vaults),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

// --- Deterministic keygen vectors -----------------------------------------------------
//
// These author only the deterministic `input.seed` (a fixed lowercase-hex 32-byte seed); the
// generator fills `expected.keypair` by running the seed-deterministic keygen (the canonical wallet
// KDF — no RNG). The comparison mode is the default **bytes**: the seed path is RNG-free, so the
// derived `{secret, public_key}` is byte-stable. Account and view share the same seed but derive under
// distinct branch labels, so their keypairs differ — both vectors prove that.

/// The committed keygen vectors. Each authors only the deterministic `input.seed`; the generator owns
/// `expected.keypair`.
const KEYGEN_VECTORS: &[VectorCase] = &[
    ("keys/account_from_seed.json", keygen_account_seed),
    ("keys/view_from_seed.json", keygen_view_seed),
];

/// The fixed 32-byte seed both keygen vectors derive from (lowercase hex; arbitrary but stable).
const KEYGEN_SEED_HEX: &str = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

/// Builds a keygen fixture shell for `name`/`operation` over the shared seed (generator fills
/// `expected.keypair`).
fn keygen_fixture_seed(name: &str, operation: &str) -> Fixture {
    Fixture {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: operation.to_string(),
        input: VectorInput {
            seed: Some(KEYGEN_SEED_HEX.to_string()),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// Account keygen vector: `seed → {account_secret, account_public_key}`.
fn keygen_account_seed() -> Fixture {
    keygen_fixture_seed("keys/account_from_seed", harness::OP_DERIVE_ACCOUNT_KEY_FROM_SEED)
}

/// View keygen vector: `seed → {view_secret, view_public_key}`.
fn keygen_view_seed() -> Fixture {
    keygen_fixture_seed("keys/view_from_seed", harness::OP_DERIVE_VIEW_KEY_FROM_SEED)
}

// --- Account-address derivation vectors (the lost-funds vector) ------------------------
//
// `account_public_key → component_<hex>`, byte-stable (`"bytes"`): the derivation is a
// domain-separated Blake2b hash with no RNG. Each authors only `input.account_public_key`; the
// generator fills `expected.component_address` by calling the public `derive_account_address` (which
// calls the same engine derivation the live transfer builder uses).
//
// Cross-check (so the vectors prove correctness, not just self-consistency): the first vector uses the
// exact raw recipient public key bytes (`0707…07`) that the committed `public_transfer` /
// `resolve_public_transfer` vectors carry as their transfer recipient. The live builder derives that
// recipient's account component from those same bytes via
// `derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, pk)` — the identical engine call
// — so `address_assert_matches_builder_recipient_derivation` below asserts this vector reproduces that
// independent builder derivation byte-for-byte. The other two vectors lock additional pks (the
// `G·scalar(7)` on-curve key the builder tests use, and the seed-derived account pubkey from
// `keys/account_from_seed.json`).

/// The committed account-address vectors. Each authors only `input.account_public_key`; the generator
/// owns `expected.component_address`.
const ADDRESS_DERIVE_VECTORS: &[VectorCase] = &[
    ("address_derive/from_recipient_pk.json", address_recipient_pk),
    ("address_derive/from_curve_pk.json", address_curve_pk),
    ("address_derive/from_seed_account_pk.json", address_seed_account_pk),
];

/// The raw recipient public-key bytes the `public_transfer` vectors carry (`0707…07`) — the
/// cross-check anchor against the live transfer-builder recipient derivation.
const ADDRESS_RECIPIENT_PK_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";

/// The seed-derived account public key from `keys/account_from_seed.json` (links the keygen and
/// address-derive vector groups: the keygen output feeds straight into address derivation).
const ADDRESS_SEED_ACCOUNT_PK_HEX: &str = "f6f89e316e6ba5f05e5250ddd4a5d3ed39dcd038cf812cc6a154b6ec0951d25f";

/// Builds an address-derive fixture shell over `pk_hex` (generator fills `expected.component_address`).
fn address_fixture_seed(name: &str, pk_hex: &str) -> Fixture {
    Fixture {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: OP_DERIVE_ACCOUNT_ADDRESS.to_string(),
        input: VectorInput {
            account_public_key: Some(pk_hex.to_string()),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// Vector 1 — the raw `0707…07` recipient pk shared with the public-transfer vectors (cross-check).
fn address_recipient_pk() -> Fixture {
    address_fixture_seed("address_derive/from_recipient_pk", ADDRESS_RECIPIENT_PK_HEX)
}

/// Vector 2 — the on-curve `G·scalar(7)` public key the builder tests use.
fn address_curve_pk() -> Fixture {
    let mut b = [0u8; 32];
    b[0] = 7;
    let sk = RistrettoSecretKey::from_canonical_bytes(&b).expect("canonical low scalar");
    let pk = RistrettoPublicKey::from_secret_key(&sk);
    address_fixture_seed("address_derive/from_curve_pk", &hex::encode(pk.as_bytes()))
}

/// Vector 3 — the seed-derived account public key (links keygen → address derivation).
fn address_seed_account_pk() -> Fixture {
    address_fixture_seed("address_derive/from_seed_account_pk", ADDRESS_SEED_ACCOUNT_PK_HEX)
}

// --- Address codec vectors (substate parse + otl_ identity bytes ↔ bech32m) ------------
//
// Two operations, one address surface:
//   * `format_identity_address` — `{network, account_key, view_only_key, pay_ref?}` → `otl_…` bech32m, byte-stable
//     (`"bytes"`). Multi-network vectors lock the network-qualified HRP; ±pay_ref vectors lock the optional 64-byte
//     memo. The keys are distinct so a field swap would change the encoded bytes.
//   * `parse_address` — a `component_/resource_<hex>` substate id or an `otl_…` identity string → the kind-tagged
//     `ParsedAddress`, byte-stable. The identity-parse vector's `input.address` is the exact string the matching
//     `format_identity_address` vector emits (the same crate codec a wallet uses), so parse cross-checks that a
//     wallet-shaped `otl_…` round-trips its fields.

/// The committed address-codec vectors (format + parse).
const ADDRESS_CODEC_VECTORS: &[VectorCase] = &[
    ("address_codec/identity_mainnet.json", identity_mainnet),
    ("address_codec/identity_esmeralda.json", identity_esmeralda),
    (
        "address_codec/identity_localnet_with_pay_ref.json",
        identity_localnet_pay_ref,
    ),
    ("address_codec/parse_identity_esmeralda.json", parse_identity_esmeralda),
    ("address_codec/parse_component.json", parse_component),
    ("address_codec/parse_resource.json", parse_resource),
];

/// The deterministic account public key the identity vectors use (the seed-derived account pubkey
/// from `keys/account_from_seed.json` — links the keygen and address-codec groups, and is a real
/// on-curve key a wallet would carry).
const IDENTITY_ACCOUNT_PK_HEX: &str = "f6f89e316e6ba5f05e5250ddd4a5d3ed39dcd038cf812cc6a154b6ec0951d25f";

/// The deterministic view-only public key the identity vectors use (the seed-derived VIEW pubkey
/// from the same seed; distinct from the account key, so a field swap would be observable).
const IDENTITY_VIEW_PK_HEX: &str = "06f832c34f9c91611d7e0b3eeb85a39f55e8f20798ae30adebec385e83983746";

/// A small, stable lowercase-hex pay_ref (a memo) for the ±pay_ref vector.
const IDENTITY_PAY_REF_HEX: &str = "00112233445566778899aabbccddeeff";

/// Builds a `format_identity_address` fixture shell (generator fills `expected.bech32m`).
fn identity_format_fixture(name: &str, network: Network, pay_ref: Option<&str>) -> Fixture {
    Fixture {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: OP_FORMAT_IDENTITY_ADDRESS.to_string(),
        input: VectorInput {
            network: Some(network),
            account_public_key: Some(IDENTITY_ACCOUNT_PK_HEX.to_string()),
            view_only_key: Some(IDENTITY_VIEW_PK_HEX.to_string()),
            pay_ref: pay_ref.map(str::to_string),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// Builds a `parse_address` fixture shell over `address` (generator fills `expected.parsed_address`).
fn parse_address_fixture(name: &str, address: String) -> Fixture {
    Fixture {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: OP_PARSE_ADDRESS.to_string(),
        input: VectorInput {
            address: Some(address),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// Identity vector — MainNet (`otl_` HRP), no pay_ref.
fn identity_mainnet() -> Fixture {
    identity_format_fixture("address_codec/identity_mainnet", Network::MainNet, None)
}

/// Identity vector — Esmeralda (`otl_esm_` HRP), no pay_ref.
fn identity_esmeralda() -> Fixture {
    identity_format_fixture("address_codec/identity_esmeralda", Network::Esmeralda, None)
}

/// Identity vector — LocalNet (`otl_loc_` HRP) with a pay_ref (the optional memo path).
fn identity_localnet_pay_ref() -> Fixture {
    identity_format_fixture(
        "address_codec/identity_localnet_with_pay_ref",
        Network::LocalNet,
        Some(IDENTITY_PAY_REF_HEX),
    )
}

/// Parse vector — the EXACT `otl_…` string the Esmeralda identity vector emits (cross-check: a
/// wallet-shaped identity string parses back to its `{network, account_key, view_only_key}` fields).
fn parse_identity_esmeralda() -> Fixture {
    let address = ootle_sdk_core::format_identity_address(
        Network::Esmeralda,
        &PublicKeyBytes::from_hex(IDENTITY_ACCOUNT_PK_HEX).expect("account pk hex"),
        &PublicKeyBytes::from_hex(IDENTITY_VIEW_PK_HEX).expect("view pk hex"),
        None,
    )
    .expect("format identity address");
    parse_address_fixture("address_codec/parse_identity_esmeralda", address)
}

/// Parse vector — a `component_<hex>` substate id (the canonical engine string round-trips).
fn parse_component() -> Fixture {
    let component = ComponentAddress::new(ObjectKey::from_array([0x22; ObjectKey::LENGTH])).to_string();
    parse_address_fixture("address_codec/parse_component", component)
}

/// Parse vector — a `resource_<hex>` substate id (the canonical engine string round-trips).
fn parse_resource() -> Fixture {
    let resource = ResourceAddress::new(ObjectKey::from_array([0x11; ObjectKey::LENGTH])).to_string();
    parse_address_fixture("address_codec/parse_resource", resource)
}

// --- Arg-DSL vectors -------------------------------------------------------------------
//
// Each vector locks the literal CBOR bytes `encode_arg` produces for one `ArgValue`, byte-for-byte.
// This fixture group protects most directly against the lost-funds drift class: a host that re-ports
// the literal encoder and gets a byte wrong fails here. The Rust unit tests in
// `src/types/generic_intent.rs` cross-check these same encodings against the builder's own
// `InstructionArg::from_type` seam, so the vectors are anchored to the engine's wire format.
const ARG_DSL_VECTORS: &[VectorCase] = &[
    ("arg_dsl/amount.json", arg_amount),
    ("arg_dsl/amount_above_2_pow_33.json", arg_amount_large),
    ("arg_dsl/u64.json", arg_u64),
    ("arg_dsl/u64_above_2_pow_33.json", arg_u64_large),
    ("arg_dsl/i64_positive.json", arg_i64_positive),
    ("arg_dsl/i64_negative.json", arg_i64_negative),
    ("arg_dsl/i64_min.json", arg_i64_min),
    ("arg_dsl/string.json", arg_string),
    ("arg_dsl/bool_true.json", arg_bool_true),
    ("arg_dsl/bool_false.json", arg_bool_false),
    ("arg_dsl/bytes.json", arg_bytes),
    ("arg_dsl/metadata.json", arg_metadata),
    ("arg_dsl/address_component.json", arg_address_component),
    ("arg_dsl/address_resource.json", arg_address_resource),
    ("arg_dsl/address_vault.json", arg_address_vault),
    ("arg_dsl/address_non_fungible.json", arg_address_non_fungible),
    (
        "arg_dsl/address_transaction_receipt.json",
        arg_address_transaction_receipt,
    ),
    ("arg_dsl/address_template.json", arg_address_template),
    (
        "arg_dsl/address_validator_fee_pool.json",
        arg_address_validator_fee_pool,
    ),
    ("arg_dsl/address_utxo.json", arg_address_utxo),
    ("arg_dsl/address_tombstone.json", arg_address_tombstone),
    ("arg_dsl/nfid_uuid.json", arg_nfid_uuid),
    ("arg_dsl/nfid_str.json", arg_nfid_str),
    ("arg_dsl/nfid_u32.json", arg_nfid_u32),
    ("arg_dsl/nfid_u64.json", arg_nfid_u64),
    ("arg_dsl/list_of_nfids.json", arg_list_of_nfids),
    ("arg_dsl/list_of_addresses.json", arg_list_of_addresses),
    ("arg_dsl/optional_address.json", arg_optional_address),
    ("arg_dsl/list_empty.json", arg_list_empty),
    ("arg_dsl/list_nested.json", arg_list_nested),
    ("arg_dsl/optional_some.json", arg_optional_some),
    ("arg_dsl/optional_none.json", arg_optional_none),
];

/// Builds an `encode_arg` fixture shell (generator fills `expected.encoded_arg_bytes`).
fn encode_arg_fixture(name: &str, arg: ArgValue) -> Fixture {
    Fixture {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        compare: harness::default_compare(),
        provenance: current_provenance(),
        operation: OP_ENCODE_ARG.to_string(),
        input: VectorInput {
            arg_value: Some(arg),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// Arg vector — a modest µTari `Amount`.
fn arg_amount() -> Fixture {
    encode_arg_fixture("arg_dsl/amount", ArgValue::Amount(1_000_000))
}

/// Arg vector — an `Amount` `> 2^33` (proves no float truncation on the amount path).
fn arg_amount_large() -> Fixture {
    encode_arg_fixture("arg_dsl/amount_above_2_pow_33", ArgValue::Amount((1u64 << 34) + 7))
}

/// Arg vector — a plain `u64` literal.
fn arg_u64() -> Fixture {
    encode_arg_fixture("arg_dsl/u64", ArgValue::U64(42))
}

/// Arg vector — a `u64` `> 2^33` (the u64-safe boundary).
fn arg_u64_large() -> Fixture {
    encode_arg_fixture("arg_dsl/u64_above_2_pow_33", ArgValue::U64((1u64 << 34) + 99))
}

/// Arg vector — a positive signed `i64` literal.
fn arg_i64_positive() -> Fixture {
    encode_arg_fixture("arg_dsl/i64_positive", ArgValue::I64(42))
}

/// Arg vector — a negative signed `i64` literal (emits a CBOR negative integer).
fn arg_i64_negative() -> Fixture {
    encode_arg_fixture("arg_dsl/i64_negative", ArgValue::I64(-42))
}

/// Arg vector — `i64::MIN`, the most-negative signed value (the CBOR negative-integer edge).
fn arg_i64_min() -> Fixture {
    encode_arg_fixture("arg_dsl/i64_min", ArgValue::I64(i64::MIN))
}

/// Arg vector — a UTF-8 string literal (e.g. a method name carried as a value arg).
fn arg_string() -> Fixture {
    encode_arg_fixture("arg_dsl/string", ArgValue::String("withdraw".to_string()))
}

/// Arg vector — boolean `true`.
fn arg_bool_true() -> Fixture {
    encode_arg_fixture("arg_dsl/bool_true", ArgValue::Bool(true))
}

/// Arg vector — boolean `false`.
fn arg_bool_false() -> Fixture {
    encode_arg_fixture("arg_dsl/bool_false", ArgValue::Bool(false))
}

/// Arg vector — raw bytes literal.
fn arg_bytes() -> Fixture {
    encode_arg_fixture("arg_dsl/bytes", ArgValue::Bytes(vec![0xde, 0xad, 0xbe, 0xef]))
}

/// Arg vector — a metadata map of strings (encoded as the engine Metadata, `BorTag<_, 129>`).
fn arg_metadata() -> Fixture {
    let map = std::collections::BTreeMap::from([
        (
            "provider_name".to_string(),
            ArgValue::String("OotleExample".to_string()),
        ),
        ("website".to_string(), ArgValue::String("example.test".to_string())),
    ]);
    encode_arg_fixture("arg_dsl/metadata", ArgValue::Metadata(map))
}

/// Arg vector — a `component_<hex>` address (encoded as the typed engine ComponentAddress).
fn arg_address_component() -> Fixture {
    let component = ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH])).to_string();
    encode_arg_fixture("arg_dsl/address_component", ArgValue::Address(component))
}

/// Arg vector — a `resource_<hex>` address (encoded as the typed engine ResourceAddress).
fn arg_address_resource() -> Fixture {
    let resource = ResourceAddress::new(ObjectKey::from_array([0xbb; ObjectKey::LENGTH])).to_string();
    encode_arg_fixture("arg_dsl/address_resource", ArgValue::Address(resource))
}

/// A fixed sample resource address used to compose non-fungible / utxo addresses.
fn arg_sample_resource() -> ResourceAddress {
    ResourceAddress::new(ObjectKey::from_array([0xcc; ObjectKey::LENGTH]))
}

/// Arg vector — a `vault_<hex>` address (encoded as the typed engine VaultId).
fn arg_address_vault() -> Fixture {
    let vault = VaultId::new(ObjectKey::from_array([0xdd; ObjectKey::LENGTH])).to_string();
    encode_arg_fixture("arg_dsl/address_vault", ArgValue::Address(vault))
}

/// Arg vector — a `nft_<resource>_<id>` address (encoded as the typed engine NonFungibleAddress).
fn arg_address_non_fungible() -> Fixture {
    let nft = NonFungibleAddress::new(arg_sample_resource(), NonFungibleId::try_from_string("nft-1").unwrap());
    encode_arg_fixture("arg_dsl/address_non_fungible", ArgValue::Address(nft.to_string()))
}

/// Arg vector — a `txreceipt_<hex>` address (encoded as the typed engine TransactionReceiptAddress).
fn arg_address_transaction_receipt() -> Fixture {
    let addr = TransactionReceiptAddress::from_array([0xee; ObjectKey::LENGTH]).to_string();
    encode_arg_fixture("arg_dsl/address_transaction_receipt", ArgValue::Address(addr))
}

/// Arg vector — a `template_<hex>` address (encoded as the untagged `TemplateAddress` hash a template parameter takes).
fn arg_address_template() -> Fixture {
    let addr = tari_engine_types::published_template::PublishedTemplateAddress::from_hash(Hash32::from_array(
        [0x12; Hash32::LENGTH],
    ))
    .to_string();
    encode_arg_fixture("arg_dsl/address_template", ArgValue::Address(addr))
}

/// Arg vector — a `vnfp_<hex>` address (encoded as the typed engine ValidatorFeePoolAddress).
fn arg_address_validator_fee_pool() -> Fixture {
    let addr = ValidatorFeePoolAddress::from_array([0x34; ObjectKey::LENGTH]).to_string();
    encode_arg_fixture("arg_dsl/address_validator_fee_pool", ArgValue::Address(addr))
}

/// Arg vector — a `utxo_<resource>_<id>` address (encoded as the typed engine UtxoAddress).
fn arg_address_utxo() -> Fixture {
    let addr = UtxoAddress::new(arg_sample_resource(), UtxoId::from_array([0x56; UtxoId::LENGTH])).to_string();
    encode_arg_fixture("arg_dsl/address_utxo", ArgValue::Address(addr))
}

/// Arg vector — a `tombstone_<hex>` address (encoded as the typed engine ClaimedOutputTombstoneAddress).
fn arg_address_tombstone() -> Fixture {
    let addr = ClaimedOutputTombstoneAddress::new(ObjectKey::from_array([0x78; ObjectKey::LENGTH])).to_string();
    encode_arg_fixture("arg_dsl/address_tombstone", ArgValue::Address(addr))
}

/// Arg vector — a `uuid_<hex>` non-fungible id (the U256 form encodes as a 32-byte CBOR byte string).
fn arg_nfid_uuid() -> Fixture {
    let id = NonFungibleId::from_u256([0x5a; 32]).to_canonical_string();
    encode_arg_fixture("arg_dsl/nfid_uuid", ArgValue::NonFungibleId(id))
}

/// Arg vector — a `str_<text>` non-fungible id.
fn arg_nfid_str() -> Fixture {
    let id = NonFungibleId::try_from_string("special-nft")
        .unwrap()
        .to_canonical_string();
    encode_arg_fixture("arg_dsl/nfid_str", ArgValue::NonFungibleId(id))
}

/// Arg vector — a `u32_<n>` non-fungible id.
fn arg_nfid_u32() -> Fixture {
    let id = NonFungibleId::from_u32(7).to_canonical_string();
    encode_arg_fixture("arg_dsl/nfid_u32", ArgValue::NonFungibleId(id))
}

/// Arg vector — a `u64_<n>` non-fungible id (`> 2^33`, proving no float truncation on the id path).
fn arg_nfid_u64() -> Fixture {
    let id = NonFungibleId::from_u64((1u64 << 34) + 5).to_canonical_string();
    encode_arg_fixture("arg_dsl/nfid_u64", ArgValue::NonFungibleId(id))
}

/// Arg vector — a `Vec<NonFungibleId>` (CBOR array of typed ids), the NFT-mint id-list shape.
fn arg_list_of_nfids() -> Fixture {
    let a = NonFungibleId::from_u32(7).to_canonical_string();
    let b = NonFungibleId::from_u64((1u64 << 34) + 5).to_canonical_string();
    encode_arg_fixture(
        "arg_dsl/list_of_nfids",
        ArgValue::List(vec![ArgValue::NonFungibleId(a), ArgValue::NonFungibleId(b)]),
    )
}

/// Arg vector — a list of addresses (each lowered to its inner typed engine address, not the wrapper).
fn arg_list_of_addresses() -> Fixture {
    let component = ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH])).to_string();
    let resource = ResourceAddress::new(ObjectKey::from_array([0xbb; ObjectKey::LENGTH])).to_string();
    encode_arg_fixture(
        "arg_dsl/list_of_addresses",
        ArgValue::List(vec![ArgValue::Address(component), ArgValue::Address(resource)]),
    )
}

/// Arg vector — `Optional(Some(address))` (the inner typed address, no wrapper).
fn arg_optional_address() -> Fixture {
    let component = ComponentAddress::new(ObjectKey::from_array([0xaa; ObjectKey::LENGTH])).to_string();
    encode_arg_fixture(
        "arg_dsl/optional_address",
        ArgValue::Optional(Some(Box::new(ArgValue::Address(component)))),
    )
}

/// Arg vector — an empty list (encodes as an empty CBOR array).
fn arg_list_empty() -> Fixture {
    encode_arg_fixture("arg_dsl/list_empty", ArgValue::List(vec![]))
}

/// Arg vector — a nested list of lists (exercises recursive array assembly).
fn arg_list_nested() -> Fixture {
    encode_arg_fixture(
        "arg_dsl/list_nested",
        ArgValue::List(vec![
            ArgValue::List(vec![ArgValue::U64(1), ArgValue::U64(2)]),
            ArgValue::List(vec![ArgValue::U64(3)]),
        ]),
    )
}

/// Arg vector — `Optional(Some(_))` (the inner value, no wrapper).
fn arg_optional_some() -> Fixture {
    encode_arg_fixture(
        "arg_dsl/optional_some",
        ArgValue::Optional(Some(Box::new(ArgValue::U64(7)))),
    )
}

/// Arg vector — `Optional(None)` (encodes as CBOR null).
fn arg_optional_none() -> Fixture {
    encode_arg_fixture("arg_dsl/optional_none", ArgValue::Optional(None))
}

// --- Generic-builder vectors (instructions → encoded tx) -------------------------------
//
// These author the deterministic `input` for `build_and_encode_instructions`: a
// `GenericTransactionIntent` (the generic instruction front-end) + a fetched batch (or explicit
// inputs) + pinned keys. The generator fills `expected` with the byte-for-byte sealed bytes/id. One
// vector per instruction kind (call method, create account, call function, publish template, a
// workspace pipe). The deterministic seal path is byte-stable, so `"compare":"bytes"` (the same
// precedent as the resolve_public_transfer group). `git_rev` is pinned to a fixed value to avoid
// churn against the pre-existing fixtures (the idempotency test pins both sides anyway).
//
// The `call_method` vector is cross-checked against the equivalent hand-built `TransactionBuilder`
// output in `generic_seals_byte_identically_to_hand_built` below.

/// Pinned git rev for the generic-build fixtures so committing them does not churn `git_rev`.
const GENERIC_BUILD_GIT_REV: &str = "78a836162980f2ddc1e18fd537f4542fc851f6a4";

const GENERIC_BUILD_VECTORS: &[VectorCase] = &[
    ("generic_build/call_method_transfer.json", generic_call_method_seed),
    ("generic_build/create_account.json", generic_create_account_seed),
    ("generic_build/call_function.json", generic_call_function_seed),
    ("generic_build/publish_template.json", generic_publish_template_seed),
    (
        "generic_build/publish_template_with_metadata.json",
        generic_publish_template_with_metadata_seed,
    ),
    ("generic_build/workspace_pipe.json", generic_workspace_pipe_seed),
    (
        "generic_build/self_funding_faucet.json",
        generic_self_funding_faucet_seed,
    ),
    ("generic_build/faucet_claim.json", faucet_claim_seed),
];

/// A `FromAccount` fee paid from the generic from-account.
fn generic_fee_from_account() -> FeeSource {
    FeeSource::FromAccount(ComponentAddressStr::from_internal(&generic_from_component()))
}

/// The from-account / fee account the generic vectors pay from + call.
fn generic_from_component() -> ComponentAddress {
    ComponentAddress::new(ObjectKey::from_array([0x71; ObjectKey::LENGTH]))
}

/// The non-TARI resource the call_method transfer withdraws.
fn generic_resource() -> ResourceAddress {
    ResourceAddress::new(ObjectKey::from_array([0x72; ObjectKey::LENGTH]))
}

/// The recipient public key for the create-account vectors.
fn generic_recipient_pk() -> PublicKeyBytes {
    let pk = RistrettoPublicKey::from_secret_key(
        &RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(73)).expect("low scalar is canonical"),
    );
    PublicKeyBytes::from_bytes(pk.as_bytes()).expect("32-byte pk")
}

/// The pinned deterministic key bundle the generic vectors seal with.
fn generic_keys() -> VectorKeys {
    VectorKeys {
        account_secret: SecretKeyBytes::from_array(fixed_scalar_bytes(74)),
    }
}

/// One explicit input (so the generic vectors resolve in a single empty `apply`, keeping the fetched
/// machinery out of the byte-stability story — the seal bytes are what these vectors lock).
fn generic_explicit_inputs() -> Vec<InputRef> {
    vec![InputRef::versioned(generic_from_component().to_string(), 0)]
}

/// Builds a `build_and_encode_instructions` fixture shell (generator fills `expected`).
fn generic_build_fixture(name: &str, intent: GenericTransactionIntent) -> Fixture {
    Fixture {
        name: name.to_string(),
        schema_version: SCHEMA_VERSION,
        compare: "semantic".to_string(),
        provenance: provenance_with_rev(GENERIC_BUILD_GIT_REV),
        operation: OP_BUILD_AND_ENCODE_INSTRUCTIONS.to_string(),
        input: VectorInput {
            network: Some(Network::Esmeralda),
            generic_intent: Some(intent),
            keys: Some(generic_keys()),
            // Explicit inputs ⇒ single empty fetched batch resolves immediately.
            fetched: Some(vec![]),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// CallMethod vector — the expressible public-transfer shape: withdraw → put-on-workspace →
/// create-account. Cross-checked against the hand-built builder below.
fn generic_transfer_intent() -> GenericTransactionIntent {
    GenericTransactionIntent {
        fee: BoundaryAmount::new(2500),
        fee_payment: generic_fee_from_account(),
        fee_instructions: vec![],
        instructions: vec![
            InstructionSpec::CallMethod {
                call: ComponentRef::Address(ComponentAddressStr::from_internal(&generic_from_component())),
                method: "withdraw".to_string(),
                args: vec![
                    ArgValue::Address(ResourceAddressStr::from_internal(&generic_resource()).0),
                    ArgValue::Amount(1_000_000),
                ],
            },
            InstructionSpec::PutLastInstructionOutputOnWorkspace {
                key: "bucket".to_string(),
            },
            InstructionSpec::CreateAccount {
                owner_public_key: generic_recipient_pk(),
                owner_rule: None,
                bucket_workspace_id: None,
            },
        ],
        blobs: vec![],
        inputs: generic_explicit_inputs(),
        extra_inputs: vec![],
        min_epoch: Some(1),
        max_epoch: 10,
        dry_run: false,
    }
}

fn generic_call_method_seed() -> Fixture {
    generic_build_fixture("generic_build/call_method_transfer", generic_transfer_intent())
}

fn generic_create_account_seed() -> Fixture {
    let intent = GenericTransactionIntent {
        fee: BoundaryAmount::new(2000),
        fee_payment: generic_fee_from_account(),
        fee_instructions: vec![],
        instructions: vec![InstructionSpec::CreateAccount {
            owner_public_key: generic_recipient_pk(),
            owner_rule: None,
            bucket_workspace_id: None,
        }],
        blobs: vec![],
        inputs: generic_explicit_inputs(),
        extra_inputs: vec![],
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
    };
    generic_build_fixture("generic_build/create_account", intent)
}

fn generic_call_function_seed() -> Fixture {
    // A faucet-style template function call (no args). The template address is a fixed 32-byte hash.
    let template = tari_template_lib_types::Hash32::from_array([0x77; 32]).to_string();
    let intent = GenericTransactionIntent {
        fee: BoundaryAmount::new(2000),
        fee_payment: generic_fee_from_account(),
        fee_instructions: vec![],
        instructions: vec![InstructionSpec::CallFunction {
            template_address: template,
            function: "take_free_coins".to_string(),
            args: vec![],
        }],
        blobs: vec![],
        inputs: generic_explicit_inputs(),
        extra_inputs: vec![],
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
    };
    generic_build_fixture("generic_build/call_function", intent)
}

fn generic_publish_template_seed() -> Fixture {
    let intent = GenericTransactionIntent {
        fee: BoundaryAmount::new(2000),
        fee_payment: generic_fee_from_account(),
        fee_instructions: vec![],
        instructions: vec![InstructionSpec::PublishTemplate {
            blob_index: 0,
            metadata_hash: None,
        }],
        blobs: vec![BlobSpec {
            bytes: vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
        }],
        inputs: generic_explicit_inputs(),
        extra_inputs: vec![],
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
    };
    generic_build_fixture("generic_build/publish_template", intent)
}

/// A publish carrying a SHA-256 multihash metadata reference (code `0x12`, length `0x20`, 32-byte digest).
fn generic_publish_template_with_metadata_seed() -> Fixture {
    let intent = GenericTransactionIntent {
        fee: BoundaryAmount::new(2000),
        fee_payment: generic_fee_from_account(),
        fee_instructions: vec![],
        instructions: vec![InstructionSpec::PublishTemplate {
            blob_index: 0,
            metadata_hash: Some(format!("1220{}", "ab".repeat(32))),
        }],
        blobs: vec![BlobSpec {
            bytes: vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00],
        }],
        inputs: generic_explicit_inputs(),
        extra_inputs: vec![],
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
    };
    generic_build_fixture("generic_build/publish_template_with_metadata", intent)
}

fn generic_workspace_pipe_seed() -> Fixture {
    // withdraw → put-on-workspace("b") → deposit(Workspace("b")): the workspace pipe encodes the ref
    // to the id the producer registered (id 0).
    let intent = GenericTransactionIntent {
        fee: BoundaryAmount::new(2000),
        fee_payment: generic_fee_from_account(),
        fee_instructions: vec![],
        instructions: vec![
            InstructionSpec::CallMethod {
                call: ComponentRef::Address(ComponentAddressStr::from_internal(&generic_from_component())),
                method: "withdraw".to_string(),
                args: vec![
                    ArgValue::Address(ResourceAddressStr::from_internal(&generic_resource()).0),
                    ArgValue::Amount(500_000),
                ],
            },
            InstructionSpec::PutLastInstructionOutputOnWorkspace { key: "b".to_string() },
            InstructionSpec::CallMethod {
                call: ComponentRef::Address(ComponentAddressStr::from_internal(&generic_from_component())),
                method: "deposit".to_string(),
                args: vec![ArgValue::Workspace("b".to_string())],
            },
        ],
        blobs: vec![],
        inputs: generic_explicit_inputs(),
        extra_inputs: vec![],
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
    };
    generic_build_fixture("generic_build/workspace_pipe", intent)
}

/// Self-funding faucet vector — the headline new capability. The fee phase creates a fresh account,
/// puts it on the workspace, has the faucet `take` fund it, and the fee is charged from that
/// workspace component. No `fee_account` exists on-ledger; the fee derives no vault want. The faucet
/// component is targeted by address (its vaults are wanted); the account is workspace-only (no want).
fn generic_self_funding_faucet_seed() -> Fixture {
    let faucet = ComponentAddress::new(ObjectKey::from_array([0x79; ObjectKey::LENGTH]));
    let intent = GenericTransactionIntent {
        fee: BoundaryAmount::new(2000),
        fee_payment: FeeSource::FromWorkspaceComponent {
            label: "faucet_account".to_string(),
        },
        fee_instructions: vec![
            InstructionSpec::CreateAccount {
                owner_public_key: generic_recipient_pk(),
                owner_rule: None,
                bucket_workspace_id: None,
            },
            InstructionSpec::PutLastInstructionOutputOnWorkspace {
                key: "faucet_account".to_string(),
            },
            InstructionSpec::CallMethod {
                call: ComponentRef::Address(ComponentAddressStr::from_internal(&faucet)),
                method: "take".to_string(),
                args: vec![ArgValue::Workspace("faucet_account".to_string())],
            },
        ],
        instructions: vec![],
        blobs: vec![],
        // Explicit inputs keep the vector byte-stable (the faucet substates are not modelled here).
        inputs: generic_explicit_inputs(),
        extra_inputs: vec![],
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
    };
    generic_build_fixture("generic_build/self_funding_faucet", intent)
}

/// First-class faucet claim vector. The core builds the complete self-funding claim with the real
/// `XTR_FAUCET_*` addresses; the fetched batch supplies the faucet component + its vault (the claim
/// resource rides in on `extra_inputs`), so it resolves in one round and seals byte-stable.
fn faucet_claim_seed() -> Fixture {
    use tari_template_lib_types::constants::{TARI_TOKEN, XTR_FAUCET_COMPONENT_ADDRESS, XTR_FAUCET_VAULT_ADDRESS};

    let faucet_vault = Vault::new(ResourceContainer::public_fungible(
        TARI_TOKEN,
        Amount::new(1_000_000_000),
    ));
    let fetched = vec![
        FetchedSubstate {
            substate_id: SubstateId::Component(XTR_FAUCET_COMPONENT_ADDRESS).to_string(),
            version: 0,
            substate_value: resolve_component_json(&[XTR_FAUCET_VAULT_ADDRESS]),
        },
        FetchedSubstate {
            substate_id: SubstateId::Vault(XTR_FAUCET_VAULT_ADDRESS).to_string(),
            version: 0,
            substate_value: indexer_substate_json(&SubstateValue::Vault(faucet_vault)),
        },
    ];

    let intent = FaucetClaimIntent {
        recipient_public_key: generic_recipient_pk(),
        fee: BoundaryAmount::new(2000),
        min_epoch: None,
        max_epoch: 1,
        dry_run: false,
    };

    Fixture {
        name: "generic_build/faucet_claim".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: "semantic".to_string(),
        provenance: provenance_with_rev(GENERIC_BUILD_GIT_REV),
        operation: OP_BUILD_AND_ENCODE_FAUCET_CLAIM.to_string(),
        input: VectorInput {
            network: Some(Network::Esmeralda),
            faucet_intent: Some(intent),
            keys: Some(generic_keys()),
            fetched: Some(fetched),
            ..Default::default()
        },
        expected: harness::ExpectedOutput::default(),
    }
}

/// Cross-check (the superset proof): the `call_method_transfer` generic vector seals to the SAME
/// bytes/id as the equivalent instruction sequence hand-built on the builder's own methods + sealed
/// through the same leaves. This anchors the generic front-end to the bespoke flow's output.
#[test]
fn generic_seals_identically_to_hand_built() {
    use ootle_sdk_core::{
        PartialTransaction,
        Resolution,
        apply_fetched_substates,
        build_unsigned_instructions_with_wants,
        decode_and_canonicalize_sealed_transfer,
        seal_and_encode_public_transfer,
    };
    use tari_ootle_transaction::{TransactionBuilder, args};

    let intent = generic_transfer_intent();
    let keys = generic_keys().to_core();

    // Generic front-end path → resolve → seal.
    let (gpartial, _w) = build_unsigned_instructions_with_wants(Network::Esmeralda, &intent).unwrap();
    let generic = match apply_fetched_substates(gpartial, &[]).unwrap() {
        Resolution::Resolved(p) => seal_and_encode_public_transfer(p, &keys).unwrap(),
        Resolution::NeedMore { .. } => panic!("expected Resolved"),
    };

    // Hand-built path: the identical instruction sequence + the same seal/encode leaves. Inputs are
    // carried ONLY by `new_with_explicit_inputs` (the generic path also never calls `with_inputs`),
    // so the comparison is like-for-like.
    let explicit_input = generic_explicit_inputs()[0].to_internal().unwrap();
    let unsigned = TransactionBuilder::new(Network::Esmeralda.as_byte(), tari_ootle_common_types::Epoch(10))
        .pay_fee_from_component(generic_from_component(), Amount::new(2500))
        .call_method(generic_from_component(), "withdraw", args![
            generic_resource(),
            Amount::new(1_000_000)
        ])
        .put_last_instruction_output_on_workspace("bucket")
        .create_account(generic_recipient_pk().to_internal())
        .with_min_epoch(Some(tari_ootle_common_types::Epoch(1)))
        .build_unsigned();
    let partial = PartialTransaction::new_with_explicit_inputs(unsigned, vec![explicit_input]);
    let hand = match apply_fetched_substates(partial, &[]).unwrap() {
        Resolution::Resolved(p) => seal_and_encode_public_transfer(p, &keys).unwrap(),
        Resolution::NeedMore { .. } => panic!("expected Resolved"),
    };

    // The signatures are random, so compare the SEMANTIC decoded transactions (the byte-unstable
    // Schnorr scalars nulled, signer public keys retained): the instruction/input encoding must be
    // byte-identical between the generic front-end and the hand-built builder.
    let g = decode_and_canonicalize_sealed_transfer(Network::Esmeralda, &generic.encoded_transaction.to_hex()).unwrap();
    let h = decode_and_canonicalize_sealed_transfer(Network::Esmeralda, &hand.encoded_transaction.to_hex()).unwrap();
    assert_eq!(
        g, h,
        "generic vector must lower + seal to the identical transaction as the hand-built builder (modulo random \
         signatures)"
    );
}

/// Cross-check: the seed-derived VIEW public key the identity vectors hard-code must equal what the
/// core actually derives from `keys/account_from_seed.json`'s seed under the view branch. Pinning the
/// pubkey as a const keeps the fixture seed builders RNG-free and self-documenting, but this guards
/// the const against drift (a wrong const would otherwise silently lock a wrong identity address).
#[test]
fn identity_view_pk_matches_seed_derivation() {
    let seed_bytes = hex::decode(KEYGEN_SEED_HEX).expect("seed hex");
    let seed: [u8; 32] = seed_bytes.try_into().expect("32-byte seed");
    let view = ootle_sdk_core::derive_view_keypair_from_seed(&seed).expect("derive view keypair");
    assert_eq!(
        view.public_key.to_hex(),
        IDENTITY_VIEW_PK_HEX,
        "IDENTITY_VIEW_PK_HEX must equal the seed-derived view public key"
    );
    // And the account const equals the account branch derivation (it is the same value the
    // keys/account_from_seed.json vector records).
    let account = ootle_sdk_core::derive_account_keypair_from_seed(&seed).expect("derive account keypair");
    assert_eq!(
        account.public_key.to_hex(),
        IDENTITY_ACCOUNT_PK_HEX,
        "IDENTITY_ACCOUNT_PK_HEX must equal the seed-derived account public key"
    );
}

/// Independent cross-check (the lost-funds guard): the `from_recipient_pk` vector's derived address
/// must equal what the live transfer builder derives for the same raw recipient bytes via the engine
/// call `derive_component_address_from_public_key`. A self-consistent but wrong public derivation is
/// exactly the failure this asserts away.
#[test]
fn address_assert_matches_builder_recipient_derivation() {
    use tari_engine_types::component::derive_component_address_from_public_key;
    use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

    let raw = hex::decode(ADDRESS_RECIPIENT_PK_HEX).expect("hex");
    let raw: [u8; 32] = raw.try_into().expect("32 bytes");

    // The independent engine derivation the builder performs for a recipient public key.
    let builder_component =
        derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &RistrettoPublicKeyBytes::from(raw));

    // The public boundary fn (what the vector / FFI exposes).
    let pk = PublicKeyBytes::from_array(raw);
    let public_component = ootle_sdk_core::derive_account_address(&pk).expect("derive");

    assert_eq!(
        public_component,
        ComponentAddressStr::from_internal(&builder_component),
        "derive_account_address must reproduce the live builder's engine derivation byte-for-byte"
    );
}

// --- Co-sign vectors ------------------------------------------------------------------
//
// One vector over a single resolved public transfer (reusing the `resolve_*` material): A builds and
// ships the unsigned record; B authorizes it (committing to A's seal pk) with a random nonce; A seals
// with that authorization attached. The `cosign_seal_with_auth` vector locks the deterministic decoded
// fields semantically (any sealed-tx vector that carries attached signatures uses "semantic", matching
// the stealth send), and its decode re-verifies B's authorization.

/// Pinned git rev for the co-sign fixtures so committing them does not churn `git_rev`.
const COSIGN_GIT_REV: &str = "78a836162980f2ddc1e18fd537f4542fc851f6a4";

const COSIGN_VECTORS: &[VectorCase] = &[("cosign/seal_with_auth.json", cosign_seal_with_auth_seed)];

/// Party A's account secret (the seal signer) for the co-sign vectors.
fn cosign_a_account_secret() -> SecretKeyBytes {
    SecretKeyBytes::from_array(fixed_scalar_bytes(121))
}

/// Party A's seal public key (hex), derived from A's account secret.
fn cosign_a_seal_pk_hex() -> String {
    let sk = RistrettoSecretKey::from_canonical_bytes(&fixed_scalar_bytes(121)).expect("low scalar is canonical");
    let pk = RistrettoPublicKey::from_secret_key(&sk);
    hex::encode(pk.as_bytes())
}

/// A's deterministic key bundle (single-key: A's account key seals).
fn cosign_a_keys() -> VectorKeys {
    VectorKeys {
        account_secret: cosign_a_account_secret(),
    }
}

/// The shared co-sign `input`: a resolved public-transfer intent (reusing the `resolve_*` material) +
/// A's keys + B's pinned co-sign key/nonce + A's seal pk. Both co-sign ops consume the same input.
fn cosign_input() -> VectorInput {
    let intent = PublicTransferIntent {
        from_account: ComponentAddressStr::from_internal(&resolve_from_component()),
        recipient: TransferRecipient::PublicKey(PublicKeyBytes::from_bytes(resolve_recipient_pk().as_bytes()).unwrap()),
        resource_address: ResourceAddressStr::from_internal(&resolve_resource()),
        amount: BoundaryAmount::new(1_000_000),
        fee: BoundaryAmount::new(2500),
        inputs: vec![],
        min_epoch: Some(1),
        max_epoch: 10,
        dry_run: false,
    };
    VectorInput {
        network: Some(Network::Esmeralda),
        intent: Some(intent),
        keys: Some(cosign_a_keys()),
        fetched: Some(resolve_fetched_batch()),
        cosign_seal_pk: Some(cosign_a_seal_pk_hex()),
        // Party B's co-signer key (distinct from A); B authorizes with a random nonce.
        cosign_signer_secret: Some(SecretKeyBytes::from_array(fixed_scalar_bytes(123))),
        ..Default::default()
    }
}

/// The `cosign/seal_with_auth` fixture shell (generator fills `expected.sealed_transaction_semantic`).
fn cosign_seal_with_auth_seed() -> Fixture {
    Fixture {
        name: "cosign/seal_with_auth".to_string(),
        schema_version: SCHEMA_VERSION,
        compare: "semantic".to_string(),
        provenance: provenance_with_rev(COSIGN_GIT_REV),
        operation: OP_COSIGN_SEAL_WITH_AUTH.to_string(),
        input: cosign_input(),
        expected: harness::ExpectedOutput::default(),
    }
}

/// Generator (gated). Seeds the sample + real vectors if missing, then refreshes `expected` +
/// `provenance` for every fixture. No-ops loudly (prints a hint) unless `OOTLE_REGEN_FIXTURES=1`.
#[test]
fn regen_fixtures() {
    if std::env::var("OOTLE_REGEN_FIXTURES").as_deref() != Ok("1") {
        eprintln!(
            "regen_fixtures: skipped (set OOTLE_REGEN_FIXTURES=1 to regenerate). The runner still checks committed \
             fixtures."
        );
        return;
    }

    // Resolve provenance ONCE for this run, so every file written in this pass shares the same
    // git_rev (and we never shell `git` more than once per regeneration).
    let provenance = current_provenance();

    // 1. Seed any vector (sample + real) that does not yet exist (create the file so pass 2 picks it up). Each entry is
    //    `(absolute path, fixture-seed builder)`.
    let mut seeds: Vec<(PathBuf, fn() -> Fixture)> = vec![(fixtures_dir().join(SAMPLE_REL_PATH), sample_fixture_seed)];
    for (rel, seed) in REAL_VECTORS
        .iter()
        .chain(RESOLVE_VECTORS.iter())
        .chain(PARSE_VECTORS.iter())
        .chain(STEALTH_OUTPUTS_VECTORS.iter())
        .chain(STEALTH_TRANSFER_VECTORS.iter())
        .chain(STEALTH_SCAN_VECTORS.iter())
        .chain(STEALTH_DECODE_VECTORS.iter())
        .chain(SUBSTATE_DECODE_VECTORS.iter())
        .chain(ACCOUNT_BALANCES_VECTORS.iter())
        .chain(KEYGEN_VECTORS.iter())
        .chain(ADDRESS_DERIVE_VECTORS.iter())
        .chain(ADDRESS_CODEC_VECTORS.iter())
        .chain(ARG_DSL_VECTORS.iter())
        .chain(GENERIC_BUILD_VECTORS.iter())
        .chain(COSIGN_VECTORS.iter())
    {
        seeds.push((fixtures_dir().join(rel), *seed));
    }

    let mut seeded_paths: Vec<PathBuf> = Vec::new();
    for (path, seed) in &seeds {
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture group dir");
        }
        let (_, json) = regenerate_with_provenance(seed(), provenance.clone());
        fs::write(path, json).expect("write seeded fixture");
        eprintln!("regen_fixtures: seeded {}", path.display());
        seeded_paths.push(path.clone());
    }

    // 2. Refresh every fixture's expected + provenance from the core. (Skip files we just seeded this run — they
    //    already hold the final, identical bytes — so each file is written once.)
    for (path, fixture) in load_all_fixtures() {
        if seeded_paths.contains(&path) {
            continue;
        }
        let (_, json) = regenerate_with_provenance(fixture, provenance.clone());
        fs::write(&path, json).expect("write regenerated fixture");
        eprintln!("regen_fixtures: wrote {}", path.display());
    }
}

/// Runner (always on). Asserts every committed fixture reproduces byte-for-byte.
#[test]
#[allow(clippy::too_many_lines)] // one linear runner over every fixture group; clearer inline
fn run_golden_vectors() {
    let fixtures = load_all_fixtures();
    assert!(
        !fixtures.is_empty(),
        "no golden-vector fixtures found under {} — run the generator (OOTLE_REGEN_FIXTURES=1)",
        fixtures_dir().display()
    );

    for (path, fixture) in &fixtures {
        assert_eq!(
            fixture.schema_version,
            SCHEMA_VERSION,
            "fixture `{}` ({}) has schema_version {}, runner expects {SCHEMA_VERSION}",
            fixture.name,
            path.display(),
            fixture.schema_version,
        );

        let actual = run_operation(fixture);

        if fixture.operation == OP_BUILD_STEALTH_OUTPUTS_STATEMENT {
            // SEMANTIC compare: the aggregated bulletproof is byte-unstable, so we
            // (1) re-validate the freshly built statement cryptographically, and (2) compare the
            // deterministic fields (statement with agg_range_proof nulled + the aggregated mask).
            assert_eq!(
                fixture.compare, "semantic",
                "stealth fixture `{}` must declare \"compare\": \"semantic\"",
                fixture.name
            );

            // (1) cryptographic validation: rebuild the real statement and validate it.
            let input = &fixture.input;
            let network = input.network.expect("stealth fixture has a network");
            let intent = input.stealth_intent.as_ref().expect("stealth fixture has an intent");
            let seed = input.stealth_seed.expect("stealth fixture has a seed");
            let (stmt, _mask) =
                ootle_sdk_core::stealth::build_stealth_outputs_statement_with_seed(network, intent, &seed, None)
                    .unwrap_or_else(|e| panic!("stealth fixture `{}`: build failed: {e}", fixture.name));
            // If any output carries a resource view key, the validator needs it (the viewable-balance
            // proof is checked against it). The vectors use a single resource view key per statement.
            let view_key = intent
                .outputs
                .iter()
                .find_map(|o| o.resource_view_key.as_ref())
                .map(|k| RistrettoPublicKey::from_canonical_bytes(k.as_bytes()).expect("canonical resource view key"));
            tari_engine_types::stealth::validate_stealth_outputs_statement(&stmt, view_key.as_ref()).unwrap_or_else(
                |e| {
                    panic!(
                        "stealth fixture `{}`: validate_stealth_outputs_statement rejected the built statement: {e:?}",
                        fixture.name
                    )
                },
            );

            // (2) deterministic-field compare.
            let expected_stmt =
                harness::canonicalize_json(fixture.expected.stealth_outputs_statement.clone().unwrap_or_else(|| {
                    panic!(
                        "stealth fixture `{}` missing expected.stealth_outputs_statement",
                        fixture.name
                    )
                }));
            let actual_stmt = actual
                .stealth_outputs_statement
                .clone()
                .unwrap_or_else(|| panic!("stealth op produced no statement for `{}`", fixture.name));
            assert_eq!(
                actual_stmt,
                expected_stmt,
                "\n*** golden-vector MISMATCH: stealth_outputs_statement (deterministic fields) ***\n fixture: {}\n    file: {}\nexpected: {}\n  actual: {}\n",
                fixture.name,
                path.display(),
                serde_json::to_string_pretty(&expected_stmt).unwrap(),
                serde_json::to_string_pretty(&actual_stmt).unwrap(),
            );
            assert_eq!(
                actual.aggregated_output_mask,
                fixture.expected.aggregated_output_mask,
                "\n*** golden-vector MISMATCH: aggregated_output_mask ***\n fixture: {}\n    file: {}\n",
                fixture.name,
                path.display(),
            );
            continue;
        }

        if fixture.operation == OP_BUILD_AND_ENCODE_STEALTH_TRANSFER {
            // SEMANTIC compare: the embedded bulletproof + balance-proof signature are
            // byte-unstable, and the seal/auth signatures sign a digest over them, so the sealed bytes
            // are not reproducible. The runner (1) re-builds + re-seals + re-validates EVERY signature
            // on the freshly sealed transaction, and (2) compares the decoded transaction structurally
            // with the byte-unstable fields (proofs + signature scalars) nulled.
            assert_eq!(
                fixture.compare, "semantic",
                "stealth-send fixture `{}` must declare \"compare\": \"semantic\"",
                fixture.name
            );

            let input = &fixture.input;
            let network = input.network.expect("stealth-send fixture has a network");
            let intent = input
                .stealth_intent
                .as_ref()
                .expect("stealth-send fixture has an intent");
            let keys = input.stealth_keys.as_ref().expect("stealth-send fixture has keys");
            let seed = keys.seed;
            let fetched = input.fetched.as_deref().unwrap_or(&[]);

            // (1) Re-build, re-seal, and validate every signature on the freshly sealed transaction.
            let out = ootle_sdk_core::build_and_encode_stealth_transfer_with_seed(
                network,
                intent,
                fetched,
                &input.spend_secrets,
                &keys.to_core(),
                &seed,
            )
            .unwrap_or_else(|e| panic!("stealth-send fixture `{}`: build failed: {e}", fixture.name));
            let tx = tari_ootle_transaction::TransactionEnvelope::from_raw(
                out.encoded_transaction.as_bytes().to_vec().into_boxed_slice(),
            )
            .decode()
            .unwrap_or_else(|e| {
                panic!(
                    "stealth-send fixture `{}`: sealed bytes must decode: {e:?}",
                    fixture.name
                )
            });
            assert!(
                tx.verify_all_signatures(),
                "stealth-send fixture `{}`: every signature on the sealed transaction must verify",
                fixture.name
            );

            // (2) deterministic-field compare (proofs + signature scalars nulled).
            let expected =
                harness::canonicalize_json(fixture.expected.sealed_transaction_semantic.clone().unwrap_or_else(|| {
                    panic!(
                        "stealth-send fixture `{}` missing expected.sealed_transaction_semantic",
                        fixture.name
                    )
                }));
            let actual_tx = actual
                .sealed_transaction_semantic
                .clone()
                .unwrap_or_else(|| panic!("stealth-send op produced no decoded tx for `{}`", fixture.name));
            assert_eq!(
                actual_tx,
                expected,
                "\n*** golden-vector MISMATCH: sealed_transaction (deterministic fields) ***\n fixture: {}\n    file: \
                 {}\nexpected: {}\n  actual: {}\n",
                fixture.name,
                path.display(),
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual_tx).unwrap(),
            );
            continue;
        }

        // Public-transfer / resolve / generic-instruction / faucet / cosign-seal all produce a sealed
        // transaction signed with random nonces, so they compare SEMANTICALLY like the stealth send:
        // `run_operation` decodes + re-verifies every signature (via
        // `decode_and_canonicalize_sealed_transfer`, which errors on a verify failure) and nulls the
        // byte-unstable Schnorr scalars, then the runner compares the deterministic decoded fields
        // (instructions, inputs, signer public keys, `main_signer` — everything except the
        // nulled scalars).
        const SEALED_SEMANTIC_OPS: &[&str] = &[
            OP_BUILD_AND_ENCODE_PUBLIC_TRANSFER,
            OP_RESOLVE_AND_ENCODE_PUBLIC_TRANSFER,
            OP_BUILD_AND_ENCODE_INSTRUCTIONS,
            OP_BUILD_AND_ENCODE_FAUCET_CLAIM,
            OP_COSIGN_SEAL_WITH_AUTH,
        ];
        if SEALED_SEMANTIC_OPS.contains(&fixture.operation.as_str()) {
            assert_eq!(
                fixture.compare, "semantic",
                "sealed-tx fixture `{}` must declare \"compare\": \"semantic\"",
                fixture.name
            );
            let expected =
                harness::canonicalize_json(fixture.expected.sealed_transaction_semantic.clone().unwrap_or_else(|| {
                    panic!(
                        "sealed-tx fixture `{}` missing expected.sealed_transaction_semantic",
                        fixture.name
                    )
                }));
            let actual_tx = actual
                .sealed_transaction_semantic
                .clone()
                .unwrap_or_else(|| panic!("sealed-tx op produced no decoded tx for `{}`", fixture.name));
            assert_eq!(
                actual_tx,
                expected,
                "\n*** golden-vector MISMATCH: sealed_transaction (deterministic fields) ***\n fixture: {}\n  file: \
                 {}\nexpected: {}\n  actual: {}\n",
                fixture.name,
                path.display(),
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual_tx).unwrap(),
            );
            continue;
        }

        if fixture.operation == OP_SCAN_STEALTH_OUTPUT {
            // BYTES compare on a structured value: the scan output is RNG-free (decryption is the
            // deterministic inverse of encryption), so the produced `DecryptedOutput` (or the
            // not-mine `null` sentinel) is byte-stable. The runner asserts the produced value equals
            // the committed `expected.decrypted` exactly.
            assert_eq!(
                fixture.compare, "bytes",
                "stealth-scan fixture `{}` must use the default \"bytes\" compare",
                fixture.name
            );
            assert_eq!(
                actual.decrypted,
                fixture.expected.decrypted,
                "\n*** golden-vector MISMATCH: scan_stealth_output decrypted ***\n fixture: {}\n    file: \
                 {}\nexpected: {}\n  actual: {}\n",
                fixture.name,
                path.display(),
                serde_json::to_string_pretty(&fixture.expected.decrypted).unwrap(),
                serde_json::to_string_pretty(&actual.decrypted).unwrap(),
            );
            continue;
        }

        if fixture.operation == OP_PARSE_FINALIZED_RESULT {
            // PARSE vectors compare a CANONICALIZED STRUCTURE, not raw bytes (unlike the encode
            // vectors): there is no CBOR byte stream here, and the parsed JSON's key order is not
            // significant. The runner canonicalizes both sides (object keys sorted) before comparing.
            let expected = harness::canonicalize_json(
                fixture
                    .expected
                    .parsed
                    .clone()
                    .unwrap_or_else(|| panic!("parse fixture `{}` missing expected.parsed", fixture.name)),
            );
            let actual_parsed = actual
                .parsed
                .clone()
                .unwrap_or_else(|| panic!("parse op produced no parsed output for `{}`", fixture.name));
            assert_eq!(
                actual_parsed,
                expected,
                "\n*** golden-vector MISMATCH: parsed FinalizedResult ***\n fixture: {}\n    file: {}\nexpected: \
                 {}\n  actual: {}\n",
                fixture.name,
                path.display(),
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual_parsed).unwrap(),
            );
            continue;
        }

        if fixture.operation == OP_DERIVE_ACCOUNT_KEY_FROM_SEED || fixture.operation == OP_DERIVE_VIEW_KEY_FROM_SEED {
            // BYTES compare on a structured value: the seed path is RNG-free (the canonical KDF), so
            // the derived `{secret, public_key}` keypair is byte-stable. Compared as a JSON object
            // (the hex fields inside are the actual byte-exact assertion).
            assert_eq!(
                fixture.compare, "bytes",
                "keygen fixture `{}` must use the default \"bytes\" compare",
                fixture.name
            );
            let expected = fixture
                .expected
                .keypair
                .clone()
                .unwrap_or_else(|| panic!("keygen fixture `{}` missing expected.keypair", fixture.name));
            let actual_kp = actual
                .keypair
                .clone()
                .unwrap_or_else(|| panic!("keygen op produced no keypair for `{}`", fixture.name));
            assert_eq!(
                actual_kp,
                expected,
                "\n*** golden-vector MISMATCH: keygen keypair ***\n fixture: {}\n    file: {}\nexpected: {}\n  \
                 actual: {}\n",
                fixture.name,
                path.display(),
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual_kp).unwrap(),
            );
            continue;
        }

        if fixture.operation == OP_DERIVE_ACCOUNT_ADDRESS {
            // BYTES compare on the canonical `component_<hex>` string: the derivation is an RNG-free
            // domain-separated hash, so the address is byte-stable. This is the lost-funds vector.
            assert_eq!(
                fixture.compare, "bytes",
                "address-derive fixture `{}` must use the default \"bytes\" compare",
                fixture.name
            );
            let expected = fixture.expected.component_address.clone().unwrap_or_else(|| {
                panic!(
                    "address-derive fixture `{}` missing expected.component_address",
                    fixture.name
                )
            });
            let actual_addr = actual
                .component_address
                .clone()
                .unwrap_or_else(|| panic!("address-derive op produced no component_address for `{}`", fixture.name));
            assert_eq!(
                actual_addr,
                expected,
                "\n*** golden-vector MISMATCH: derived component_address ***\n fixture: {}\n    file: {}\nexpected: \
                 {}\n  actual: {}\n",
                fixture.name,
                path.display(),
                expected,
                actual_addr,
            );
            continue;
        }

        if fixture.operation == OP_FORMAT_IDENTITY_ADDRESS {
            // BYTES compare on the canonical `otl_…` bech32m string: encoding is RNG-free, so the
            // string is byte-stable. The multi-network vectors lock the network-qualified HRP.
            assert_eq!(
                fixture.compare, "bytes",
                "format-identity fixture `{}` must use the default \"bytes\" compare",
                fixture.name
            );
            let expected = fixture
                .expected
                .bech32m
                .clone()
                .unwrap_or_else(|| panic!("format-identity fixture `{}` missing expected.bech32m", fixture.name));
            let actual_bech32m = actual
                .bech32m
                .clone()
                .unwrap_or_else(|| panic!("format-identity op produced no bech32m for `{}`", fixture.name));
            assert_eq!(
                actual_bech32m,
                expected,
                "\n*** golden-vector MISMATCH: format_identity_address bech32m ***\n fixture: {}\n    file: \
                 {}\nexpected: {}\n  actual: {}\n",
                fixture.name,
                path.display(),
                expected,
                actual_bech32m,
            );
            continue;
        }

        if fixture.operation == OP_PARSE_ADDRESS {
            // BYTES compare on the kind-tagged ParsedAddress structure: parsing is RNG-free, so the
            // produced record is byte-stable. Compared as a canonicalized JSON object.
            assert_eq!(
                fixture.compare, "bytes",
                "parse-address fixture `{}` must use the default \"bytes\" compare",
                fixture.name
            );
            let expected = harness::canonicalize_json(fixture.expected.parsed_address.clone().unwrap_or_else(|| {
                panic!(
                    "parse-address fixture `{}` missing expected.parsed_address",
                    fixture.name
                )
            }));
            let actual_parsed = actual
                .parsed_address
                .clone()
                .unwrap_or_else(|| panic!("parse-address op produced no parsed_address for `{}`", fixture.name));
            assert_eq!(
                actual_parsed,
                expected,
                "\n*** golden-vector MISMATCH: parse_address ***\n fixture: {}\n    file: {}\nexpected: {}\n  actual: \
                 {}\n",
                fixture.name,
                path.display(),
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual_parsed).unwrap(),
            );
            continue;
        }

        if fixture.operation == OP_DECODE_STEALTH_UTXO {
            // BYTES compare on the decoded InboundStealthOutput structure: the decode is a pure parse
            // + field map (no RNG), so the produced record is byte-stable. Compared as a canonicalized
            // JSON object.
            assert_eq!(
                fixture.compare, "bytes",
                "decode-stealth-utxo fixture `{}` must use the default \"bytes\" compare",
                fixture.name
            );
            let expected = harness::canonicalize_json(fixture.expected.inbound_output.clone().unwrap_or_else(|| {
                panic!(
                    "decode-stealth-utxo fixture `{}` missing expected.inbound_output",
                    fixture.name
                )
            }));
            let actual_inbound = actual.inbound_output.clone().unwrap_or_else(|| {
                panic!(
                    "decode-stealth-utxo op produced no inbound_output for `{}`",
                    fixture.name
                )
            });
            assert_eq!(
                actual_inbound,
                expected,
                "\n*** golden-vector MISMATCH: decode_stealth_utxo ***\n fixture: {}\n    file: {}\nexpected: {}\n  \
                 actual: {}\n",
                fixture.name,
                path.display(),
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual_inbound).unwrap(),
            );
            continue;
        }

        if fixture.operation == harness::OP_DECODE_SUBSTATE {
            // BYTES compare on the kind-tagged DecodedSubstate structure: the decode is RNG-free, so
            // the record is byte-stable. The embedded u64 balances are native JSON numbers.
            assert_eq!(
                fixture.compare, "bytes",
                "decode-substate fixture `{}` must use the default \"bytes\" compare",
                fixture.name
            );
            let expected = harness::canonicalize_json(fixture.expected.decoded_substate.clone().unwrap_or_else(|| {
                panic!(
                    "decode-substate fixture `{}` missing expected.decoded_substate",
                    fixture.name
                )
            }));
            let actual_decoded = actual
                .decoded_substate
                .clone()
                .unwrap_or_else(|| panic!("decode-substate op produced no decoded_substate for `{}`", fixture.name));
            assert_eq!(
                actual_decoded,
                expected,
                "\n*** golden-vector MISMATCH: decode_substate ***\n fixture: {}\n    file: {}\nexpected: {}\n  \
                 actual: {}\n",
                fixture.name,
                path.display(),
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual_decoded).unwrap(),
            );
            continue;
        }

        if fixture.operation == harness::OP_ACCOUNT_BALANCES {
            // BYTES compare on the Vec<ResourceBalance> structure: the sum is RNG-free, so the record
            // is byte-stable. The u64 revealed balances are native JSON numbers (a > 2^33 balance is
            // locked here, guarding against any float truncation).
            assert_eq!(
                fixture.compare, "bytes",
                "account-balances fixture `{}` must use the default \"bytes\" compare",
                fixture.name
            );
            let expected = harness::canonicalize_json(fixture.expected.account_balances.clone().unwrap_or_else(|| {
                panic!(
                    "account-balances fixture `{}` missing expected.account_balances",
                    fixture.name
                )
            }));
            let actual_balances = actual.account_balances.clone().unwrap_or_else(|| {
                panic!(
                    "account-balances op produced no account_balances for `{}`",
                    fixture.name
                )
            });
            assert_eq!(
                actual_balances,
                expected,
                "\n*** golden-vector MISMATCH: account_balances ***\n fixture: {}\n    file: {}\nexpected: {}\n  \
                 actual: {}\n",
                fixture.name,
                path.display(),
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual_balances).unwrap(),
            );
            continue;
        }

        if fixture.operation == harness::OP_ENCODE_ARG {
            // BYTE-FOR-BYTE on the literal CBOR bytes: arg encoding is RNG-free and MUST match the
            // engine's wire format exactly. A single wrong byte here is the lost-funds drift class
            // this group guards against.
            assert_eq!(
                fixture.compare, "bytes",
                "encode-arg fixture `{}` must use the default \"bytes\" compare",
                fixture.name
            );
            assert_eq!(
                actual.encoded_arg_bytes,
                fixture.expected.encoded_arg_bytes,
                "\n*** golden-vector MISMATCH: encode_arg encoded_arg_bytes ***\n fixture: {}\n    file: \
                 {}\nexpected: {}\n  actual: {}\n",
                fixture.name,
                path.display(),
                fixture.expected.encoded_arg_bytes,
                actual.encoded_arg_bytes,
            );
            continue;
        }

        // Every operation is handled by one of the arms above (each `continue`s). A fixture whose
        // operation matches none of them is a bug in the fixture or a missing runner arm.
        panic!(
            "fixture `{}` ({}): operation `{}` has no runner arm",
            fixture.name,
            path.display(),
            fixture.operation,
        );
    }
}

/// Round-trip / idempotency guard: regenerating each committed fixture produces byte-identical JSON,
/// so a committed fixture and a freshly generated one agree — generate `expected`, then prove the
/// committed file already holds exactly that.
///
/// Provenance `git_rev` is pinned to a fixed value **on both sides** (threaded as a parameter, never
/// via `std::env::set_var` — that would race the parallel test harness). Pinning isolates the compare
/// to the *bytes* (`expected` output), which is what idempotency means here: provenance metadata is
/// expected to drift with HEAD, the produced bytes are not.
#[test]
fn regen_is_idempotent() {
    let pinned: Provenance = provenance_with_rev("test-pinned-rev");

    for (path, fixture) in load_all_fixtures() {
        // Left: the committed fixture (its own `expected`) with provenance pinned to the same value.
        let mut committed_fixture = fixture.clone();
        committed_fixture.provenance = pinned.clone();
        let committed = fixture_to_pretty_json(&committed_fixture);

        // Right: regenerate `expected` from the core, with the same pinned provenance.
        let (_, regenerated) = regenerate_with_provenance(fixture, pinned.clone());

        assert_eq!(
            committed,
            regenerated,
            "regeneration is not idempotent for {} — the committed fixture is stale; rerun OOTLE_REGEN_FIXTURES=1 \
             cargo test -p ootle_sdk_core --test golden_vectors regen_fixtures",
            path.display(),
        );
    }
}

/// A from-scratch round-trip: generate a vector in a temp dir, then load it and confirm the runner's
/// operation reproduces its `expected` byte-for-byte — without touching the committed fixtures.
#[test]
fn generate_then_run_round_trip() {
    let tmp = std::env::temp_dir().join(format!("ootle_sdk_core_golden_{}", std::process::id()));
    fs::create_dir_all(&tmp).expect("create temp dir");
    let path: PathBuf = tmp.join("roundtrip.json");

    // Generate: drive the core to fill `expected`, write the JSON.
    let (generated, json) = regenerate(sample_fixture_seed());
    fs::write(&path, &json).expect("write temp fixture");

    // Run: reload from disk and re-run the operation; assert the semantic sealed transaction (the
    // sample is a public-transfer op, which seals with a random nonce and compares semantically —
    // the byte-unstable Schnorr scalars are nulled, so the decoded record is stable across runs).
    let raw = fs::read_to_string(&path).expect("read temp fixture");
    let loaded: Fixture = serde_json::from_str(&raw).expect("parse temp fixture");
    let actual = run_operation(&loaded);

    assert!(
        actual.sealed_transaction_semantic.is_some(),
        "the sample op must produce a decoded semantic transaction"
    );
    assert_eq!(
        actual.sealed_transaction_semantic, generated.expected.sealed_transaction_semantic,
        "round-trip semantic sealed transaction must match"
    );

    fs::remove_dir_all(&tmp).ok();
}
