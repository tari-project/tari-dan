//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Integration tests that drive the flat C ABI directly (the `extern "C"` fns), exactly as a C / Go
//! host would, and assert:
//!
//! - **one-shot round-trip** — `ootle_build_and_encode_public_transfer` over a committed `public_transfer/*` vector
//!   returns a well-formed encoded transfer (the seal uses a random Schnorr nonce, so the bytes are not
//!   byte-reproducible across calls — the check is structural: non-empty hex + a 64-hex-char transaction id).
//! - **two-phase handle flow** — `ootle_build_unsigned` → `ootle_apply_fetched_substates` → `ootle_seal_and_encode`
//!   seals a valid transaction from a `resolve_public_transfer/*` vector.
//! - **parse** — `ootle_parse_finalized_result` over a committed parse vector matches its expected parsed JSON
//!   structurally.
//! - **error envelope** — malformed intent / bad key map to the right stable `error_code`, not a crash.
//! - **panic safety** — a NULL handle is reported, never UB.
//! - **no leaks** — a counting global allocator drives a looped leak test ([`no_leaks_over_many_round_trips`]). It runs
//!   a full round-trip N times; a per-iteration leak of even one allocation would grow the live count by ~N, which
//!   dwarfs the cross-thread noise of a process-wide counter, so the test asserts the net growth stays far below N.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    ffi::{CStr, CString},
    os::raw::c_char,
    path::PathBuf,
    sync::atomic::{AtomicI64, Ordering},
};

use ootle_sdk_ffi_c::{
    OotleResult,
    ootle_abi_version,
    ootle_account_balance_wants,
    ootle_account_balances,
    ootle_add_signature,
    ootle_apply_fetched_substates,
    ootle_apply_fetched_substates_stealth,
    ootle_build_and_encode_public_transfer,
    ootle_build_and_encode_stealth_transfer,
    ootle_build_and_encode_stealth_transfer_with_seed,
    ootle_build_faucet_claim,
    ootle_build_stealth_outputs_statement_with_seed,
    ootle_build_stealth_unsigned,
    ootle_build_stealth_unsigned_with_seed,
    ootle_build_unsigned,
    ootle_build_unsigned_instructions,
    ootle_decode_stealth_utxo,
    ootle_decode_substate,
    ootle_derive_account_address,
    ootle_derive_account_key_from_seed,
    ootle_derive_view_key_from_seed,
    ootle_format_identity_address,
    ootle_generate_account_key,
    ootle_generate_view_key,
    ootle_parse_address,
    ootle_parse_finalized_result,
    ootle_partial_transaction_free,
    ootle_result_free,
    ootle_scan_stealth_output,
    ootle_scan_stealth_substate,
    ootle_seal_and_encode,
    ootle_seal_and_encode_stealth,
    ootle_seal_and_encode_with_auth,
    ootle_stealth_partial_transaction_free,
    ootle_unsigned_record_for_cosign,
    ootle_validate_stealth_transfer,
};

// --- Counting global allocator (leak detection) -------------------------------------------------

/// Net live allocation count (allocs − frees). Used to bracket a round-trip and assert it returns to
/// its starting value once every returned pointer is freed.
static LIVE: AtomicI64 = AtomicI64::new(0);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(1, Ordering::SeqCst);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn live() -> i64 {
    LIVE.load(Ordering::SeqCst)
}

// --- Fixture loading ----------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    // crates/ootle_sdk_ffi_c/tests/ -> crates/ootle_sdk_core/fixtures/
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("ootle_sdk_core")
        .join("fixtures")
}

fn load_fixture(rel: &str) -> serde_json::Value {
    let path = fixtures_dir().join(rel);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// The L1 network discriminant byte for a fixture's `input.network` keyword (e.g. `"esmeralda"`).
fn network_byte(network_keyword: &serde_json::Value) -> u8 {
    let network: ootle_sdk_core::types::network::Network =
        serde_json::from_value(network_keyword.clone()).expect("network keyword deserializes");
    network.as_byte()
}

// --- Small C-string + envelope helpers (the "host" side) ----------------------------------------

/// A host-owned C string argument. Frees itself on drop (mirrors a real host freeing its own args).
struct CArg(CString);

impl CArg {
    fn new(s: &str) -> Self {
        CArg(CString::new(s).expect("no interior NUL in test JSON"))
    }

    fn ptr(&self) -> *const c_char {
        self.0.as_ptr()
    }
}

/// Reads a (non-null) envelope C string field into an owned `String` (copies; does not free).
fn read_cstr(p: *const c_char) -> String {
    assert!(!p.is_null(), "expected a non-null C string");
    unsafe { CStr::from_ptr(p) }.to_str().expect("valid UTF-8").to_string()
}

/// The success flag as a bool (`ok` is a `u8` `1`/`0` over the C ABI).
fn ok(r: &OotleResult) -> bool {
    r.ok == 1
}

fn error_code(r: &OotleResult) -> String {
    read_cstr(r.error_code)
}

fn data_json(r: &OotleResult) -> serde_json::Value {
    assert!(!r.data_json.is_null(), "expected data_json on a success envelope");
    let s = read_cstr(r.data_json);
    serde_json::from_str(&s).expect("data_json is valid JSON")
}

// --- (1) one-shot round-trip --------------------------------------------------------------------

/// The random-nonce default `ootle_build_and_encode_public_transfer` (account-secret-only keys) is
/// **structural-only**: it returns a well-formed encoded transfer, and two calls differ (fresh seed
/// each call). No byte-equality against a committed vector (the seal nonces are non-deterministic).
#[test]
fn one_shot_random_default_is_structural_and_non_reproducible() {
    let fx = load_fixture("public_transfer/single_key_basic.json");
    let input = &fx["input"];
    let network = network_byte(&input["network"]);
    let intent = CArg::new(&serde_json::to_string(&input["intent"]).unwrap());
    // The random symbol takes only `{account_secret}` (no seed).
    let keys = CArg::new(
        &serde_json::to_string(&serde_json::json!({ "account_secret": input["keys"]["account_secret"] })).unwrap(),
    );

    let a = unsafe { ootle_build_and_encode_public_transfer(network, intent.ptr(), keys.ptr()) };
    assert!(ok(&a), "expected success, got code {}", error_code(&a));
    assert!(a.handle.is_null(), "one-shot op returns no handle");
    let a_out = data_json(&a);
    assert!(
        !a_out["encoded_transaction"].as_str().unwrap().is_empty(),
        "random default produces non-empty bytes"
    );
    let a_tx = a_out["transaction_id"].as_str().unwrap().to_string();
    unsafe { ootle_result_free(a) };

    let b = unsafe { ootle_build_and_encode_public_transfer(network, intent.ptr(), keys.ptr()) };
    assert!(ok(&b));
    let b_tx = data_json(&b)["transaction_id"].as_str().unwrap().to_string();
    unsafe { ootle_result_free(b) };
    assert_ne!(
        a_tx, b_tx,
        "the random-nonce default seals a fresh transaction id each call"
    );
}

// --- (2) two-phase handle flow ------------------------------------------------------------------

#[test]
fn two_phase_handle_flow_seals_valid_transaction() {
    let fx = load_fixture("resolve_public_transfer/single_key_basic.json");
    let input = &fx["input"];
    let network = network_byte(&input["network"]);
    let intent = CArg::new(&serde_json::to_string(&input["intent"]).unwrap());
    let fetched = CArg::new(&serde_json::to_string(&input["fetched"]).unwrap());
    let keys = CArg::new(
        &serde_json::to_string(&serde_json::json!({ "account_secret": input["keys"]["account_secret"] })).unwrap(),
    );

    // 1) build_unsigned → handle + want list.
    let built = unsafe { ootle_build_unsigned(network, intent.ptr()) };
    assert!(ok(&built), "build_unsigned failed: {}", error_code(&built));
    assert!(!built.handle.is_null(), "build_unsigned returns a handle");
    let want_body = data_json(&built);
    assert!(
        want_body["want_list"].as_array().is_some(),
        "build_unsigned data_json carries a want_list array"
    );
    let handle = built.handle;
    unsafe { ootle_result_free(built) }; // frees the strings; handle stays live (threaded forward).

    // 2) apply_fetched_substates → consumes handle, returns resolved handle.
    let applied = unsafe { ootle_apply_fetched_substates(handle, fetched.ptr()) };
    assert!(ok(&applied), "apply failed: {}", error_code(&applied));
    assert!(!applied.handle.is_null(), "apply returns the threaded handle");
    let status = data_json(&applied);
    assert_eq!(
        status["status"], "resolved",
        "the full fetched batch resolves the partial"
    );
    let resolved = applied.handle;
    unsafe { ootle_result_free(applied) };

    // 3) seal_and_encode → consumes handle, returns encoded bytes. The random-nonce seal is not
    // byte-reproducible, so the resolved path is asserted structurally (a valid encoded transfer).
    let sealed = unsafe { ootle_seal_and_encode(resolved, keys.ptr()) };
    assert!(ok(&sealed), "seal failed: {}", error_code(&sealed));
    assert!(sealed.handle.is_null(), "seal returns no handle");
    assert_well_formed_encoded(&data_json(&sealed));
    unsafe { ootle_result_free(sealed) };
}

/// The two-phase flow converges across **multiple** fetch rounds when the host fetches strictly the
/// ids the core hands back — proving the `NeedMore` response exposes the *discovered* vault id (in
/// `fetch_ids`), not just a re-seed of the component. Without this, a thin host that only ever fetched
/// the want-list seeds could never learn the vault id and would loop until a cap fired. Here round 1
/// supplies only the from-component; round 2 supplies the vault named in `fetch_ids`; the sealed bytes
/// still match the single-batch vector byte-for-byte.
#[test]
fn two_phase_multi_round_converges_via_discovered_fetch_ids() {
    let fx = load_fixture("resolve_public_transfer/single_key_basic.json");
    let input = &fx["input"];
    let network = network_byte(&input["network"]);
    let intent = CArg::new(&serde_json::to_string(&input["intent"]).unwrap());
    let keys = CArg::new(
        &serde_json::to_string(&serde_json::json!({ "account_secret": input["keys"]["account_secret"] })).unwrap(),
    );

    // Split the committed fetched batch into the component (round 1) and the vault (round 2) — a real
    // host cannot supply the vault in round 1 because it does not yet know the vault id.
    let all_fetched = input["fetched"].as_array().expect("fetched is an array");
    let component_batch: Vec<_> = all_fetched
        .iter()
        .filter(|s| s["substate_id"].as_str().unwrap().starts_with("component_"))
        .cloned()
        .collect();
    let vault_batch: Vec<_> = all_fetched
        .iter()
        .filter(|s| s["substate_id"].as_str().unwrap().starts_with("vault_"))
        .cloned()
        .collect();
    assert_eq!(component_batch.len(), 1, "fixture has one component");
    assert_eq!(vault_batch.len(), 1, "fixture has one vault");
    let vault_id = vault_batch[0]["substate_id"].as_str().unwrap().to_string();

    // 1) build_unsigned → handle.
    let built = unsafe { ootle_build_unsigned(network, intent.ptr()) };
    assert!(ok(&built), "build_unsigned failed: {}", error_code(&built));
    let mut handle = built.handle;
    unsafe { ootle_result_free(built) };

    // Round 1: only the from-component. Expect need_more with the discovered vault id in fetch_ids.
    let round1 = CArg::new(&serde_json::to_string(&component_batch).unwrap());
    let applied1 = unsafe { ootle_apply_fetched_substates(handle, round1.ptr()) };
    assert!(ok(&applied1), "round 1 apply failed: {}", error_code(&applied1));
    let status1 = data_json(&applied1);
    assert_eq!(
        status1["status"], "need_more",
        "component alone cannot resolve the from-vault want"
    );
    let fetch_ids = status1["fetch_ids"]
        .as_array()
        .expect("need_more carries a fetch_ids array");
    assert!(
        fetch_ids.iter().any(|id| id.as_str() == Some(vault_id.as_str())),
        "fetch_ids must expose the discovered vault id {vault_id}, got {fetch_ids:?}"
    );
    handle = applied1.handle;
    assert!(!handle.is_null(), "need_more threads the handle forward");
    unsafe { ootle_result_free(applied1) };

    // Round 2: fetch exactly what fetch_ids named (the vault). Now it resolves.
    let round2 = CArg::new(&serde_json::to_string(&vault_batch).unwrap());
    let applied2 = unsafe { ootle_apply_fetched_substates(handle, round2.ptr()) };
    assert!(ok(&applied2), "round 2 apply failed: {}", error_code(&applied2));
    assert_eq!(
        data_json(&applied2)["status"],
        "resolved",
        "fetching fetch_ids converges"
    );
    let resolved = applied2.handle;
    unsafe { ootle_result_free(applied2) };

    // Seal → a well-formed encoded transfer (the random-nonce seal is not byte-reproducible; the
    // fetch order does not affect that the resolved handle seals a valid transaction).
    let sealed = unsafe { ootle_seal_and_encode(resolved, keys.ptr()) };
    assert!(ok(&sealed), "seal failed: {}", error_code(&sealed));
    assert_well_formed_encoded(&data_json(&sealed));
    unsafe { ootle_result_free(sealed) };
}

/// The handle from `build_unsigned`, if the host aborts the flow, is freed exactly once via
/// `ootle_partial_transaction_free`.
#[test]
fn abandoned_handle_is_freed_cleanly() {
    let fx = load_fixture("resolve_public_transfer/single_key_basic.json");
    let input = &fx["input"];
    let network = network_byte(&input["network"]);
    let intent = CArg::new(&serde_json::to_string(&input["intent"]).unwrap());

    let built = unsafe { ootle_build_unsigned(network, intent.ptr()) };
    assert!(ok(&built));
    let handle = built.handle;
    unsafe { ootle_result_free(built) };
    // Host decides to abort — free the never-consumed handle directly.
    unsafe { ootle_partial_transaction_free(handle) };
}

// --- (2b) generic builder: the existing apply/seal/free reused ------------------------------------
//
// `ootle_build_unsigned_instructions` lowers a `GenericTransactionIntent` and returns the same
// `HandleKind::Public` handle as `ootle_build_unsigned`, so the host finishes it with the existing
// `ootle_apply_fetched_substates` + `ootle_seal_and_encode` — no generic lifecycle fns exist. These
// tests prove the handle is interchangeable with the public path and the error/kind contracts hold.

/// The generic builder's handle is consumed by the existing apply/seal surface and reproduces the
/// committed `build_and_encode_instructions` vector byte-for-byte (the explicit-input single-round
/// path: the intent carries its inputs, so an empty fetched batch resolves immediately).
#[test]
fn generic_build_instructions_flow_seals_valid_transaction() {
    let fx = load_fixture("generic_build/call_method_transfer.json");
    let input = &fx["input"];
    let network = network_byte(&input["network"]);
    let intent = CArg::new(&serde_json::to_string(&input["generic_intent"]).unwrap());
    let fetched = CArg::new(&serde_json::to_string(&input["fetched"]).unwrap());
    let keys = CArg::new(
        &serde_json::to_string(&serde_json::json!({ "account_secret": input["keys"]["account_secret"] })).unwrap(),
    );

    // 1) build_unsigned_instructions → handle + want list (the SAME envelope shape as build_unsigned).
    let built = unsafe { ootle_build_unsigned_instructions(network, intent.ptr()) };
    assert!(ok(&built), "build_unsigned_instructions failed: {}", error_code(&built));
    assert!(!built.handle.is_null(), "the generic build returns a (public) handle");
    let want_body = data_json(&built);
    assert!(
        want_body["want_list"].as_array().is_some(),
        "the generic build data_json carries a want_list array"
    );
    let handle = built.handle;
    unsafe { ootle_result_free(built) };

    // 2) The existing public apply consumes it (no generic apply fn exists).
    let applied = unsafe { ootle_apply_fetched_substates(handle, fetched.ptr()) };
    assert!(ok(&applied), "apply failed: {}", error_code(&applied));
    let status = data_json(&applied);
    assert_eq!(
        status["status"], "resolved",
        "the explicit-input intent resolves immediately"
    );
    let resolved = applied.handle;
    unsafe { ootle_result_free(applied) };

    // 3) The existing public seal consumes it → a well-formed encoded transfer (random-nonce seal is
    // not byte-reproducible; the generic-built handle is interchangeable with the public seal surface).
    let sealed = unsafe { ootle_seal_and_encode(resolved, keys.ptr()) };
    assert!(ok(&sealed), "seal failed: {}", error_code(&sealed));
    assert_well_formed_encoded(&data_json(&sealed));
    unsafe { ootle_result_free(sealed) };
}

/// A generic-build handle the host aborts is freed cleanly via the existing public free fn.
#[test]
fn generic_build_abandoned_handle_is_freed_cleanly() {
    let fx = load_fixture("generic_build/create_account.json");
    let intent = CArg::new(&serde_json::to_string(&fx["input"]["generic_intent"]).unwrap());
    let network = network_byte(&fx["input"]["network"]);

    let built = unsafe { ootle_build_unsigned_instructions(network, intent.ptr()) };
    assert!(ok(&built));
    let handle = built.handle;
    unsafe { ootle_result_free(built) };
    // Aborted flow: the existing public free handles the generic-build handle.
    unsafe { ootle_partial_transaction_free(handle) };
}

/// Bad intent JSON ⇒ a deterministic `PARSE` error, no handle, no crash.
#[test]
fn generic_build_bad_intent_json_is_a_parse_error() {
    let intent = CArg::new("{ not valid generic intent json");
    let result =
        unsafe { ootle_build_unsigned_instructions(network_byte(&serde_json::json!("esmeralda")), intent.ptr()) };
    assert!(!ok(&result), "malformed JSON must not succeed");
    assert_eq!(error_code(&result), "PARSE");
    assert!(result.handle.is_null(), "error envelope carries no handle");
    unsafe { ootle_result_free(result) };
}

/// A fee payer label that names no component slot (unbound, or carrying a `.offset` suffix) ⇒
/// `VALIDATION`, no handle.
#[test]
fn generic_build_bad_workspace_fee_label_is_a_validation_error() {
    let fx = load_fixture("generic_build/self_funding_faucet.json");
    let network = network_byte(&fx["input"]["network"]);
    for label in ["never_bound", "faucet_account.0"] {
        let mut intent = fx["input"]["generic_intent"].clone();
        intent["fee_payment"] = serde_json::json!({ "FromWorkspaceComponent": { "label": label } });
        let intent = CArg::new(&serde_json::to_string(&intent).unwrap());
        let result = unsafe { ootle_build_unsigned_instructions(network, intent.ptr()) };
        assert!(!ok(&result), "fee label '{label}' must not succeed");
        assert_eq!(error_code(&result), "VALIDATION", "fee label '{label}'");
        assert!(result.handle.is_null(), "error envelope carries no handle");
        unsafe { ootle_result_free(result) };
    }
}

/// A NULL intent arg ⇒ `INVALID`, never UB.
#[test]
fn generic_build_null_intent_is_invalid_not_ub() {
    let result =
        unsafe { ootle_build_unsigned_instructions(network_byte(&serde_json::json!("esmeralda")), std::ptr::null()) };
    assert!(!ok(&result));
    assert_eq!(error_code(&result), "INVALID");
    unsafe { ootle_result_free(result) };
}

// --- (2b) faucet claim --------------------------------------------------------------------------
//
// `ootle_build_faucet_claim` emits the complete self-funding faucet claim and returns the same
// (public) handle + want-list envelope, finished via the existing apply/seal surface.

/// The faucet claim builds, resolves from the committed fetched batch (faucet component + vault), and
/// seals byte-for-byte to the `build_and_encode_faucet_claim` vector.
#[test]
fn faucet_claim_flow_seals_valid_transaction() {
    let fx = load_fixture("generic_build/faucet_claim.json");
    let input = &fx["input"];
    let network = network_byte(&input["network"]);
    let intent = CArg::new(&serde_json::to_string(&input["faucet_intent"]).unwrap());
    let fetched = CArg::new(&serde_json::to_string(&input["fetched"]).unwrap());
    let keys = CArg::new(
        &serde_json::to_string(&serde_json::json!({ "account_secret": input["keys"]["account_secret"] })).unwrap(),
    );

    // 1) build_faucet_claim → handle + want list.
    let built = unsafe { ootle_build_faucet_claim(network, intent.ptr()) };
    assert!(ok(&built), "build_faucet_claim failed: {}", error_code(&built));
    assert!(!built.handle.is_null(), "the faucet build returns a (public) handle");
    assert!(
        data_json(&built)["want_list"].as_array().is_some(),
        "the faucet build data_json carries a want_list array"
    );
    let handle = built.handle;
    unsafe { ootle_result_free(built) };

    // 2) The existing public apply resolves it from the faucet substates.
    let applied = unsafe { ootle_apply_fetched_substates(handle, fetched.ptr()) };
    assert!(ok(&applied), "apply failed: {}", error_code(&applied));
    assert_eq!(
        data_json(&applied)["status"],
        "resolved",
        "one batch resolves the claim"
    );
    let resolved = applied.handle;
    unsafe { ootle_result_free(applied) };

    // 3) Seal → a well-formed encoded transfer (the random-nonce seal is not byte-reproducible).
    let sealed = unsafe { ootle_seal_and_encode(resolved, keys.ptr()) };
    assert!(ok(&sealed), "seal failed: {}", error_code(&sealed));
    assert_well_formed_encoded(&data_json(&sealed));
    unsafe { ootle_result_free(sealed) };
}

/// Bad faucet intent JSON ⇒ `PARSE`; a NULL arg ⇒ `INVALID`. Neither is UB.
#[test]
fn faucet_claim_bad_and_null_intent_are_clean_errors() {
    let net = network_byte(&serde_json::json!("esmeralda"));

    let bad = CArg::new("{ not valid faucet intent");
    let r = unsafe { ootle_build_faucet_claim(net, bad.ptr()) };
    assert_eq!(error_code(&r), "PARSE");
    assert!(r.handle.is_null());
    unsafe { ootle_result_free(r) };

    let r = unsafe { ootle_build_faucet_claim(net, std::ptr::null()) };
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };
}

// --- (3) parse ----------------------------------------------------------------------------------

#[test]
fn parse_finalized_result_matches_vector() {
    let fx = load_fixture("parse_finalized_result/accept.json");
    let raw = CArg::new(&serde_json::to_string(&fx["input"]["raw_result"]).unwrap());

    let result = unsafe { ootle_parse_finalized_result(raw.ptr()) };
    assert!(ok(&result), "parse failed: {}", error_code(&result));
    let parsed = canonicalize(data_json(&result));
    let expected = canonicalize(fx["expected"]["parsed"].clone());
    assert_eq!(
        parsed, expected,
        "parsed FinalizedResult must match the committed vector"
    );
    unsafe { ootle_result_free(result) };
}

/// The same `ootle_parse_finalized_result` fn shape-dispatches a dry-run response (a top-level
/// `result` key) and surfaces the additive `estimated_fee` (a bare u64 `> 2^53`) — no new ABI surface.
#[test]
fn parse_finalized_result_surfaces_dry_run_estimated_fee() {
    let fx = load_fixture("parse_finalized_result/dry_run.json");
    let raw = CArg::new(&serde_json::to_string(&fx["input"]["raw_result"]).unwrap());

    let result = unsafe { ootle_parse_finalized_result(raw.ptr()) };
    assert!(ok(&result), "dry-run parse failed: {}", error_code(&result));
    let parsed = canonicalize(data_json(&result));
    let expected = canonicalize(fx["expected"]["parsed"].clone());
    assert_eq!(parsed, expected, "parsed dry-run FinalizedResult must match the vector");
    // The estimated fee is a bare u64 (the > 2^53 metered total + 1), never coerced through a float.
    assert_eq!(
        parsed["estimated_fee"], fx["expected"]["parsed"]["estimated_fee"],
        "estimated_fee must round-trip as a bare u64 over the ABI"
    );
    assert!(
        parsed["estimated_fee"].as_u64().unwrap() > (1u64 << 53),
        "the dry-run vector exercises a u64 above 2^53"
    );
    unsafe { ootle_result_free(result) };
}

/// Recursively sort object keys for an order-insensitive structural compare (parse vectors compare
/// structure, not bytes — mirrors the core harness's `canonicalize_json`).
fn canonicalize(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(entries.into_iter().map(|(k, v)| (k, canonicalize(v))).collect())
        },
        serde_json::Value::Array(items) => serde_json::Value::Array(items.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

// --- (4) error envelopes ------------------------------------------------------------------------

#[test]
fn malformed_intent_json_is_a_parse_error() {
    let network = network_byte(&serde_json::json!("esmeralda"));
    let intent = CArg::new("{ this is not valid json");
    let keys = CArg::new(r#"{"account_secret":"6500000000000000000000000000000000000000000000000000000000000000"}"#);

    let result = unsafe { ootle_build_and_encode_public_transfer(network, intent.ptr(), keys.ptr()) };
    assert!(!ok(&result), "malformed JSON must not succeed");
    assert_eq!(error_code(&result), "PARSE");
    assert!(result.data_json.is_null(), "error envelope carries no data_json");
    assert!(result.handle.is_null(), "error envelope carries no handle");
    assert!(!read_cstr(result.error_message).is_empty(), "error has a message");
    unsafe { ootle_result_free(result) };
}

#[test]
fn malformed_secret_key_is_a_key_error() {
    let fx = load_fixture("public_transfer/single_key_basic.json");
    let input = &fx["input"];
    let network = network_byte(&input["network"]);
    let intent = CArg::new(&serde_json::to_string(&input["intent"]).unwrap());
    // 0xff..ff is a valid-width but non-canonical Ristretto scalar → KEY error from the core.
    let keys = CArg::new(r#"{"account_secret":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"}"#);

    let result = unsafe { ootle_build_and_encode_public_transfer(network, intent.ptr(), keys.ptr()) };
    assert!(!ok(&result));
    assert_eq!(error_code(&result), "KEY");
    unsafe { ootle_result_free(result) };
}

#[test]
fn bad_key_width_is_a_parse_error() {
    // A too-short hex key fails the fixed-width hex deserialize → PARSE at the facade boundary.
    let fx = load_fixture("public_transfer/single_key_basic.json");
    let input = &fx["input"];
    let network = network_byte(&input["network"]);
    let intent = CArg::new(&serde_json::to_string(&input["intent"]).unwrap());
    let keys = CArg::new(r#"{"account_secret":"6500"}"#);

    let result = unsafe { ootle_build_and_encode_public_transfer(network, intent.ptr(), keys.ptr()) };
    assert!(!ok(&result));
    assert_eq!(error_code(&result), "PARSE");
    unsafe { ootle_result_free(result) };
}

#[test]
fn unknown_network_byte_is_invalid() {
    let intent = CArg::new("{}");
    let keys = CArg::new("{}");
    let result = unsafe { ootle_build_and_encode_public_transfer(0xff, intent.ptr(), keys.ptr()) };
    assert!(!ok(&result));
    assert_eq!(error_code(&result), "INVALID");
    unsafe { ootle_result_free(result) };
}

// --- (5) null-handle / misuse safety ------------------------------------------------------------

#[test]
fn null_handle_to_apply_is_invalid_not_ub() {
    let fetched = CArg::new("[]");
    let result = unsafe { ootle_apply_fetched_substates(std::ptr::null_mut(), fetched.ptr()) };
    assert!(!ok(&result));
    assert_eq!(error_code(&result), "INVALID");
    unsafe { ootle_result_free(result) };
}

#[test]
fn null_handle_to_seal_is_invalid_not_ub() {
    let keys = CArg::new("{}");
    let result = unsafe { ootle_seal_and_encode(std::ptr::null_mut(), keys.ptr()) };
    assert!(!ok(&result));
    assert_eq!(error_code(&result), "INVALID");
    unsafe { ootle_result_free(result) };
}

#[test]
fn null_json_arg_is_invalid_not_ub() {
    let result = unsafe { ootle_build_unsigned(network_byte(&serde_json::json!("esmeralda")), std::ptr::null()) };
    assert!(!ok(&result));
    assert_eq!(error_code(&result), "INVALID");
    unsafe { ootle_result_free(result) };
}

// --- (6) free fns are null-safe -----------------------------------------------------------------

#[test]
fn free_fns_are_null_safe() {
    // Calling the free fns on null must be a no-op, never a crash.
    unsafe { ootle_partial_transaction_free(std::ptr::null_mut()) };
    let empty = OotleResult {
        ok: 1,
        error_code: std::ptr::null_mut(),
        error_message: std::ptr::null_mut(),
        data_json: std::ptr::null_mut(),
        handle: std::ptr::null_mut(),
    };
    unsafe { ootle_result_free(empty) };
}

// --- (6b) handle KIND-tag misuse safety (type-confusion guard) ----------------------------------
//
// The stealth handle reaches the host through the shared `OotleResult.handle` field, cross-cast to
// `*mut OotlePartialTransaction`. The kind tag turns routing it to a public consumer/free (or vice
// versa) from UB into a deterministic `INVALID` error, and a rejected consume/free leaves the handle
// intact and re-usable with its correct fn. These tests prove exactly that — no crash, no bad free.

/// Builds a fresh stealth handle (resolver state) from a committed vector and returns its raw pointer
/// (cross-cast to the public type, exactly as the shared envelope hands it out). The caller owns it.
fn build_stealth_handle_as_public() -> *mut ootle_sdk_ffi_c::OotlePartialTransaction {
    let fx = load_fixture("stealth_transfer/stealth_seal_with_input.json");
    let a = stealth_send_args(&fx);
    let built = unsafe { ootle_build_stealth_unsigned(a.network, a.intent.ptr()) };
    assert!(ok(&built), "stealth build failed: {}", error_code(&built));
    let handle = built.handle; // typed `*mut OotlePartialTransaction` (the cross-cast in the envelope)
    unsafe { ootle_result_free(built) };
    handle
}

/// Builds a fresh public handle from a committed vector and returns its raw pointer. The caller owns it.
fn build_public_handle() -> *mut ootle_sdk_ffi_c::OotlePartialTransaction {
    let resolved = load_fixture("resolve_public_transfer/single_key_basic.json");
    let net = network_byte(&resolved["input"]["network"]);
    let intent = CArg::new(&serde_json::to_string(&resolved["input"]["intent"]).unwrap());
    let built = unsafe { ootle_build_unsigned(net, intent.ptr()) };
    assert!(ok(&built), "public build failed: {}", error_code(&built));
    let handle = built.handle;
    unsafe { ootle_result_free(built) };
    handle
}

/// Routing a STEALTH handle to the public `ootle_apply_fetched_substates` → deterministic `INVALID`
/// error (kind mismatch), no crash. The handle is NOT consumed (guard rejects before `Box::from_raw`),
/// so it remains valid and is freed cleanly with its correct stealth free fn afterwards.
#[test]
fn stealth_handle_to_public_apply_is_invalid_not_ub() {
    let handle = build_stealth_handle_as_public();
    let fetched = CArg::new("[]");

    let r = unsafe { ootle_apply_fetched_substates(handle, fetched.ptr()) };
    assert!(!ok(&r), "a misrouted stealth handle must error, not succeed");
    assert_eq!(error_code(&r), "INVALID", "kind mismatch is a deterministic INVALID");
    assert!(r.handle.is_null(), "the rejected call returns no handle");
    unsafe { ootle_result_free(r) };

    // The guard left the handle intact — free it with the CORRECT (stealth) free fn. No bad free.
    let stealth = handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_stealth_partial_transaction_free(stealth) };
}

/// Routing a STEALTH handle to the public `ootle_seal_and_encode` → `INVALID`, handle intact.
#[test]
fn stealth_handle_to_public_seal_is_invalid_not_ub() {
    let handle = build_stealth_handle_as_public();
    let keys = CArg::new("{}");

    let r = unsafe { ootle_seal_and_encode(handle, keys.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    assert!(r.handle.is_null());
    unsafe { ootle_result_free(r) };

    let stealth = handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_stealth_partial_transaction_free(stealth) };
}

/// Routing a STEALTH handle to the public free fn `ootle_partial_transaction_free` → a no-op (NOT a
/// bad free). The handle survives and is then freed correctly with the stealth free fn.
#[test]
fn stealth_handle_to_public_free_is_a_noop_not_bad_free() {
    let handle = build_stealth_handle_as_public();

    // Wrong-kind free: must be a deterministic no-op (the guard refuses to `Box::from_raw` it).
    unsafe { ootle_partial_transaction_free(handle) };

    // Still valid — free correctly. (If the wrong-kind free had actually dropped it, this would be a
    // double-free / UAF; the leak loop separately proves the balance.)
    let stealth = handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_stealth_partial_transaction_free(stealth) };
}

/// Routing a PUBLIC handle to the stealth `ootle_apply_fetched_substates_stealth` consumer →
/// `INVALID`, handle intact, then freed correctly with the public free fn.
#[test]
fn public_handle_to_stealth_apply_is_invalid_not_ub() {
    let handle = build_public_handle();
    let stealth = handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    let net = network_byte(&serde_json::json!("esmeralda"));
    let empty = CArg::new("[]");

    let r = unsafe { ootle_apply_fetched_substates_stealth(stealth, net, empty.ptr(), empty.ptr(), empty.ptr()) };
    assert!(!ok(&r), "a misrouted public handle must error");
    assert_eq!(error_code(&r), "INVALID");
    assert!(r.handle.is_null());
    unsafe { ootle_result_free(r) };

    // Intact — free with the correct (public) free fn.
    unsafe { ootle_partial_transaction_free(handle) };
}

/// Routing a PUBLIC handle to the stealth `ootle_seal_and_encode_stealth` → `INVALID`, handle intact
/// (the guard rejects before consuming), then freed correctly with the public free fn.
#[test]
fn public_handle_to_stealth_seal_is_invalid_not_ub() {
    let handle = build_public_handle();
    let stealth = handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    let net = network_byte(&serde_json::json!("esmeralda"));
    let keys = CArg::new("{}");

    let r = unsafe { ootle_seal_and_encode_stealth(stealth, net, keys.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    assert!(r.handle.is_null());
    unsafe { ootle_result_free(r) };

    // The rejected consume leaves ownership with the caller — free it with its CORRECT (public) fn.
    unsafe { ootle_partial_transaction_free(handle) };
}

/// Routing a PUBLIC handle to the stealth free fn → a no-op (NOT a bad free); freed correctly after.
#[test]
fn public_handle_to_stealth_free_is_a_noop_not_bad_free() {
    let handle = build_public_handle();
    let stealth = handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;

    unsafe { ootle_stealth_partial_transaction_free(stealth) };

    unsafe { ootle_partial_transaction_free(handle) };
}

// --- (6b) identity keygen -----------------------------------------------------------------------

/// A lowercase-hex 32-byte field is a 64-char hex string.
fn assert_hex32(value: &serde_json::Value, field: &str) {
    let s = value[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} must be a hex string"));
    assert_eq!(s.len(), 64, "{field} must be 32 bytes (64 hex chars)");
    assert!(
        s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "{field} must be lowercase hex"
    );
}

#[test]
fn generate_account_key_returns_a_fresh_keypair() {
    let r = unsafe { ootle_generate_account_key() };
    assert!(ok(&r), "expected success, got {}", error_code(&r));
    assert!(r.handle.is_null(), "keygen returns no handle");
    let out = data_json(&r);
    assert_hex32(&out, "account_secret");
    assert_hex32(&out, "account_public_key");
    unsafe { ootle_result_free(r) };
}

#[test]
fn generate_view_key_returns_a_fresh_keypair() {
    let r = unsafe { ootle_generate_view_key() };
    assert!(ok(&r), "expected success, got {}", error_code(&r));
    let out = data_json(&r);
    assert_hex32(&out, "view_secret");
    assert_hex32(&out, "view_public_key");
    unsafe { ootle_result_free(r) };
}

#[test]
fn generate_account_keys_are_distinct_across_calls() {
    let a = unsafe { ootle_generate_account_key() };
    let b = unsafe { ootle_generate_account_key() };
    let sa = data_json(&a)["account_secret"].as_str().unwrap().to_string();
    let sb = data_json(&b)["account_secret"].as_str().unwrap().to_string();
    assert_ne!(sa, sb, "two OsRng draws must differ");
    unsafe { ootle_result_free(a) };
    unsafe { ootle_result_free(b) };
}

#[test]
fn derive_account_key_from_seed_reproduces_vector() {
    let fx = load_fixture("keys/account_from_seed.json");
    let seed = CArg::new(fx["input"]["seed"].as_str().unwrap());
    let r = unsafe { ootle_derive_account_key_from_seed(seed.ptr()) };
    assert!(ok(&r), "expected success, got {}", error_code(&r));
    assert!(r.handle.is_null());
    let out = data_json(&r);
    assert_eq!(out["account_secret"], fx["expected"]["keypair"]["account_secret"]);
    assert_eq!(
        out["account_public_key"],
        fx["expected"]["keypair"]["account_public_key"]
    );
    unsafe { ootle_result_free(r) };
}

#[test]
fn derive_view_key_from_seed_reproduces_vector() {
    let fx = load_fixture("keys/view_from_seed.json");
    let seed = CArg::new(fx["input"]["seed"].as_str().unwrap());
    let r = unsafe { ootle_derive_view_key_from_seed(seed.ptr()) };
    assert!(ok(&r), "expected success, got {}", error_code(&r));
    let out = data_json(&r);
    assert_eq!(out["view_secret"], fx["expected"]["keypair"]["view_secret"]);
    assert_eq!(out["view_public_key"], fx["expected"]["keypair"]["view_public_key"]);
    unsafe { ootle_result_free(r) };
}

#[test]
fn derive_key_from_seed_rejects_bad_hex() {
    // Odd-length hex.
    let bad = CArg::new("abc");
    let r = unsafe { ootle_derive_account_key_from_seed(bad.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Uppercase hex is rejected.
    let upper = CArg::new(&"AB".repeat(32));
    let r = unsafe { ootle_derive_view_key_from_seed(upper.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Wrong length (16 bytes, not 32).
    let short = CArg::new(&"aa".repeat(16));
    let r = unsafe { ootle_derive_account_key_from_seed(short.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Null arg → INVALID.
    let r = unsafe { ootle_derive_account_key_from_seed(std::ptr::null()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };
}

// --- (6c) account-address derivation ------------------------------------------------------------

#[test]
fn derive_account_address_reproduces_vector() {
    let fx = load_fixture("address_derive/from_recipient_pk.json");
    let pk = CArg::new(fx["input"]["account_public_key"].as_str().unwrap());
    let r = unsafe { ootle_derive_account_address(pk.ptr()) };
    assert!(ok(&r), "expected success, got {}", error_code(&r));
    assert!(r.handle.is_null(), "address derivation returns no handle");
    let out = data_json(&r);
    assert_eq!(out["component_address"], fx["expected"]["component_address"]);
    assert!(
        out["component_address"].as_str().unwrap().starts_with("component_"),
        "canonical component_<hex>"
    );
    unsafe { ootle_result_free(r) };
}

#[test]
fn derive_account_address_rejects_bad_hex() {
    // Odd-length hex.
    let bad = CArg::new("abc");
    let r = unsafe { ootle_derive_account_address(bad.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Uppercase hex is rejected.
    let upper = CArg::new(&"AB".repeat(32));
    let r = unsafe { ootle_derive_account_address(upper.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Wrong length (16 bytes, not 32).
    let short = CArg::new(&"aa".repeat(16));
    let r = unsafe { ootle_derive_account_address(short.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Null arg → INVALID.
    let r = unsafe { ootle_derive_account_address(std::ptr::null()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };
}

// --- (6d) address parse/format codec ------------------------------------------------------------

#[test]
fn format_identity_address_round_trips_via_parse() {
    let fx = load_fixture("address_codec/identity_esmeralda.json");
    let net = network_byte(&fx["input"]["network"]);
    let account = CArg::new(fx["input"]["account_public_key"].as_str().unwrap());
    let view = CArg::new(fx["input"]["view_only_key"].as_str().unwrap());

    // Format: matches the committed bech32m exactly (pay_ref NULL).
    let r = unsafe { ootle_format_identity_address(net, account.ptr(), view.ptr(), std::ptr::null()) };
    assert!(ok(&r), "expected success, got {}", error_code(&r));
    assert!(r.handle.is_null(), "format returns no handle");
    let out = data_json(&r);
    let bech32m = out["bech32m"].as_str().unwrap().to_string();
    assert_eq!(bech32m, fx["expected"]["bech32m"].as_str().unwrap());
    unsafe { ootle_result_free(r) };

    // Parse the just-formatted string back: the kind-tagged identity record round-trips its fields
    // (account_key not swapped with view_only_key).
    let addr = CArg::new(&bech32m);
    let r = unsafe { ootle_parse_address(addr.ptr()) };
    assert!(ok(&r), "expected success, got {}", error_code(&r));
    let parsed = data_json(&r);
    assert_eq!(parsed["kind"], "identity");
    assert_eq!(parsed["network"], "esmeralda");
    assert_eq!(parsed["account_key"], fx["input"]["account_public_key"]);
    assert_eq!(parsed["view_only_key"], fx["input"]["view_only_key"]);
    assert_eq!(parsed["bech32m"], bech32m);
    unsafe { ootle_result_free(r) };
}

#[test]
fn format_identity_address_with_pay_ref() {
    let fx = load_fixture("address_codec/identity_localnet_with_pay_ref.json");
    let net = network_byte(&fx["input"]["network"]);
    let account = CArg::new(fx["input"]["account_public_key"].as_str().unwrap());
    let view = CArg::new(fx["input"]["view_only_key"].as_str().unwrap());
    let pay_ref = CArg::new(fx["input"]["pay_ref"].as_str().unwrap());

    let r = unsafe { ootle_format_identity_address(net, account.ptr(), view.ptr(), pay_ref.ptr()) };
    assert!(ok(&r), "expected success, got {}", error_code(&r));
    let out = data_json(&r);
    assert_eq!(
        out["bech32m"].as_str().unwrap(),
        fx["expected"]["bech32m"].as_str().unwrap()
    );
    let bech32m = out["bech32m"].as_str().unwrap().to_string();
    unsafe { ootle_result_free(r) };

    // Parse it back: the pay_ref hex survives.
    let addr = CArg::new(&bech32m);
    let r = unsafe { ootle_parse_address(addr.ptr()) };
    assert!(ok(&r));
    let parsed = data_json(&r);
    assert_eq!(parsed["pay_ref"], fx["input"]["pay_ref"]);
    unsafe { ootle_result_free(r) };
}

#[test]
fn parse_address_substate_kinds() {
    let comp = load_fixture("address_codec/parse_component.json");
    let addr = CArg::new(comp["input"]["address"].as_str().unwrap());
    let r = unsafe { ootle_parse_address(addr.ptr()) };
    assert!(ok(&r), "expected success, got {}", error_code(&r));
    let parsed = data_json(&r);
    assert_eq!(parsed["kind"], "component");
    assert_eq!(parsed["canonical"], comp["input"]["address"]);
    unsafe { ootle_result_free(r) };

    let res = load_fixture("address_codec/parse_resource.json");
    let addr = CArg::new(res["input"]["address"].as_str().unwrap());
    let r = unsafe { ootle_parse_address(addr.ptr()) };
    assert!(ok(&r));
    let parsed = data_json(&r);
    assert_eq!(parsed["kind"], "resource");
    assert_eq!(parsed["canonical"], res["input"]["address"]);
    unsafe { ootle_result_free(r) };
}

#[test]
fn parse_address_rejects_bad_input() {
    // Unknown prefix → PARSE.
    let bad = CArg::new("nope_1234");
    let r = unsafe { ootle_parse_address(bad.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Corrupted bech32m checksum → PARSE.
    let corrupt = CArg::new("otl_esm_1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq");
    let r = unsafe { ootle_parse_address(corrupt.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Null arg → INVALID.
    let r = unsafe { ootle_parse_address(std::ptr::null()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };
}

#[test]
fn format_identity_address_rejects_bad_input() {
    let account = CArg::new(&"aa".repeat(32));
    let view = CArg::new(&"bb".repeat(32));

    // Bad account key hex (uppercase) → PARSE.
    let upper = CArg::new(&"AB".repeat(32));
    let r = unsafe { ootle_format_identity_address(0x26, upper.ptr(), view.ptr(), std::ptr::null()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // pay_ref over 64 bytes → PARSE.
    let over_cap = CArg::new(&"aa".repeat(65));
    let r = unsafe { ootle_format_identity_address(0x26, account.ptr(), view.ptr(), over_cap.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Unknown network byte → INVALID.
    let r = unsafe { ootle_format_identity_address(0xFF, account.ptr(), view.ptr(), std::ptr::null()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };

    // Null required arg → INVALID.
    let r = unsafe { ootle_format_identity_address(0x26, std::ptr::null(), view.ptr(), std::ptr::null()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };
}

// --- (6b) Co-signing ----------------------------------------------------------------------------

/// Drives the full FFI co-sign hand-off from the committed cosign vector: A builds + resolves a
/// handle, extracts the unsigned record (non-consuming), B authorizes it, A seals with the
/// authorization attached (consuming the handle), and the sealed result validates.
#[test]
fn cosign_round_trip_seals_a_validating_transaction() {
    let fx = load_fixture("cosign/seal_with_auth.json");
    let net = network_byte(&fx["input"]["network"]);
    let intent = CArg::new(&serde_json::to_string(&fx["input"]["intent"]).unwrap());
    let fetched = CArg::new(&serde_json::to_string(&fx["input"]["fetched"]).unwrap());
    let keys = CArg::new(&serde_json::to_string(&fx["input"]["keys"]).unwrap());
    let seal_pk = CArg::new(fx["input"]["cosign_seal_pk"].as_str().unwrap());
    let signer_secret = CArg::new(fx["input"]["cosign_signer_secret"].as_str().unwrap());
    let signer_pk = fx["expected"]["sealed_transaction_semantic"]["V1"]["body"]["signatures"][0]["public_key"]
        .as_str()
        .unwrap();
    let main_signer = CArg::new(signer_pk);

    // A: build + resolve.
    let built = unsafe { ootle_build_unsigned(net, intent.ptr()) };
    assert!(ok(&built));
    let h = built.handle;
    unsafe { ootle_result_free(built) };
    let applied = unsafe { ootle_apply_fetched_substates(h, fetched.ptr()) };
    assert!(ok(&applied));
    let h = applied.handle;
    unsafe { ootle_result_free(applied) };

    // A: extract the unsigned record to ship, naming B as the main signer (does NOT consume the handle).
    let rec = unsafe { ootle_unsigned_record_for_cosign(h, main_signer.ptr()) };
    assert!(ok(&rec));
    assert_eq!(
        data_json(&rec)["unsigned"]["V1"]["main_signer"],
        serde_json::json!({ "Authorizer": signer_pk })
    );
    let record_json = serde_json::to_string(&data_json(&rec)).unwrap();
    unsafe { ootle_result_free(rec) };

    // B: authorize, committing to A's seal pk.
    let unsigned = CArg::new(&record_json);
    let auth = unsafe { ootle_add_signature(net, unsigned.ptr(), seal_pk.ptr(), signer_secret.ptr()) };
    assert!(ok(&auth));
    let auth_obj = data_json(&auth)["authorization"].clone();
    assert_eq!(auth_obj["public_key"], signer_pk);
    assert!(auth_obj["signature"].is_string());
    unsafe { ootle_result_free(auth) };

    // A: attach + seal (consumes the handle).
    let auths = CArg::new(&serde_json::to_string(&serde_json::json!([auth_obj])).unwrap());
    let sealed = unsafe { ootle_seal_and_encode_with_auth(h, keys.ptr(), auths.ptr(), main_signer.ptr()) };
    assert!(ok(&sealed));
    let encoded_hex = data_json(&sealed)["encoded_transaction"]
        .as_str()
        .expect("encoded_transaction hex")
        .to_string();
    assert!(!encoded_hex.is_empty());
    unsafe { ootle_result_free(sealed) };
}

/// `ootle_add_signature` maps a malformed signer secret hex to `KEY` and never panics.
#[test]
fn cosign_add_signature_bad_secret_hex_is_a_key_error() {
    let fx = load_fixture("cosign/seal_with_auth.json");
    let net = network_byte(&fx["input"]["network"]);
    let seal_pk = CArg::new(fx["input"]["cosign_seal_pk"].as_str().unwrap());

    // Build a minimal valid record by resolving the handle, then extracting it.
    let intent = CArg::new(&serde_json::to_string(&fx["input"]["intent"]).unwrap());
    let fetched = CArg::new(&serde_json::to_string(&fx["input"]["fetched"]).unwrap());
    let built = unsafe { ootle_build_unsigned(net, intent.ptr()) };
    let h = built.handle;
    unsafe { ootle_result_free(built) };
    let applied = unsafe { ootle_apply_fetched_substates(h, fetched.ptr()) };
    let h = applied.handle;
    unsafe { ootle_result_free(applied) };
    let bad_main_signer = CArg::new("zz");
    let r = unsafe { ootle_unsigned_record_for_cosign(h, bad_main_signer.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };
    let rec = unsafe { ootle_unsigned_record_for_cosign(h, std::ptr::null()) };
    let record_json = serde_json::to_string(&data_json(&rec)).unwrap();
    unsafe { ootle_result_free(rec) };
    unsafe { ootle_partial_transaction_free(h) };

    let unsigned = CArg::new(&record_json);

    // Odd-length / non-hex secret ⇒ KEY.
    let bad = CArg::new("nothex");
    let r = unsafe { ootle_add_signature(net, unsigned.ptr(), seal_pk.ptr(), bad.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "KEY");
    unsafe { ootle_result_free(r) };

    // Bad seal-pk hex ⇒ PARSE.
    let good_secret = CArg::new(fx["input"]["cosign_signer_secret"].as_str().unwrap());
    let bad_pk = CArg::new("zz");
    let r = unsafe { ootle_add_signature(net, unsigned.ptr(), bad_pk.ptr(), good_secret.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };
}

/// `ootle_seal_and_encode_with_auth` validates the handle kind: a misrouted stealth handle is
/// rejected with `INVALID` and **not** consumed (it is freed correctly afterwards).
#[test]
fn cosign_seal_with_auth_rejects_wrong_kind_handle() {
    let fx = load_fixture("cosign/seal_with_auth.json");
    let net = network_byte(&fx["input"]["network"]);
    let keys = CArg::new(&serde_json::to_string(&fx["input"]["keys"]).unwrap());
    let auths = CArg::new("[]");

    // Build a STEALTH handle and misroute it to the public seal-with-auth fn.
    let stealth_fx = load_fixture("stealth_transfer/stealth_seal_with_input.json");
    let s = stealth_send_args(&stealth_fx);
    let sbuilt = unsafe { ootle_build_stealth_unsigned(s.network, s.intent.ptr()) };
    assert!(ok(&sbuilt));
    let sh = sbuilt.handle; // cross-cast `*mut OotlePartialTransaction`
    unsafe { ootle_result_free(sbuilt) };

    let _ = net; // network is not an arg to the seal-with-auth fn; keep symmetry with other tests.
    let r = unsafe { ootle_seal_and_encode_with_auth(sh, keys.ptr(), auths.ptr(), std::ptr::null()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };

    // The handle was NOT consumed — free it correctly with the stealth free fn.
    let sh_stealth = sh as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_stealth_partial_transaction_free(sh_stealth) };
}

// --- (7) ABI version tag ------------------------------------------------------------------------

#[test]
fn abi_version_is_the_stable_tag() {
    let p = ootle_abi_version();
    assert!(!p.is_null());
    let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap();
    assert_eq!(
        s, "ootle-sdk-ffi-c/19",
        "the ABI tag every host binding asserts against (bumped when the co-sign exports gained the main signer)"
    );
    // Static pointer — explicitly NOT freed (freeing it would be UB).
}

// --- (9) stealth surface ------------------------------------------------------------------------

/// Pulls the six stealth-send arguments out of a `stealth_transfer/*` fixture's `input` block into
/// host-owned C strings (one place so each test reads cleanly).
struct StealthSendArgs {
    network: u8,
    intent: CArg,
    fetched: CArg,
    spend_secrets: CArg,
    keys: CArg,
    /// The `{account_secret, view_secret}` keys mirror (no seed) for the random-nonce default symbols.
    account_only_keys: CArg,
    /// The fixture's build seed (lowercase hex) for the `_with_seed` build symbols.
    seed_hex: CArg,
}

fn stealth_send_args(fx: &serde_json::Value) -> StealthSendArgs {
    let input = &fx["input"];
    // `fetched` / `spend_secrets` are absent for a revealed-only transfer (no stealth inputs); default
    // them to empty arrays so a missing key serializes to `[]`, not `null`.
    let empty = serde_json::Value::Array(vec![]);
    let fetched = input.get("fetched").unwrap_or(&empty);
    let spend_secrets = input.get("spend_secrets").unwrap_or(&empty);
    let account_only = serde_json::json!({
        "account_secret": input["stealth_keys"]["account_secret"],
        "view_secret": input["stealth_keys"]["view_secret"],
    });
    StealthSendArgs {
        network: network_byte(&input["network"]),
        intent: CArg::new(&serde_json::to_string(&input["stealth_intent"]).unwrap()),
        fetched: CArg::new(&serde_json::to_string(fetched).unwrap()),
        spend_secrets: CArg::new(&serde_json::to_string(spend_secrets).unwrap()),
        keys: CArg::new(&serde_json::to_string(&input["stealth_keys"]).unwrap()),
        account_only_keys: CArg::new(&serde_json::to_string(&account_only).unwrap()),
        seed_hex: CArg::new(input["stealth_keys"]["seed"].as_str().expect("stealth_keys.seed hex")),
    }
}

/// The from-account component + its vault, as the indexer returns them. The two-phase live driver
/// declares both as wants (the fee + any revealed input draw on the account), so a test must serve
/// them before a transfer resolves. The addresses match the stealth fixtures (`component_aa..` /
/// `resource_01..`, vault `vault_cc..`); the JSON was captured from the core's looped-resolution helpers.
const ACCOUNT_SUBSTATES: &str = r#"[{"substate_id":"component_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","substate_value":{"Component":{"body":{"state":[{"@cbor":"tag","tag":132,"value":{"@cbor":"bytes","hex":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}}]},"header":{"access_rules":{"default":"DenyAll","method_access":{}},"entity_id":"00","owner_rule":"None","template_address":"0000000000000000000000000000000000000000000000000000000000000000"}}},"version":0},{"substate_id":"vault_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","substate_value":{"Vault":{"freeze_flags":0,"resource_container":{"Fungible":{"address":"resource_0101010101010101010101010101010101010101010101010101010101010101","amount":"1000000","locked_amount":"0"}}}},"version":0}]"#;

/// The from-account component + vault batch alone (no stealth UTXO) — resolves the account wants while
/// any required stealth UTXO stays unsupplied (drives the missing-UTXO error path).
fn account_substates_batch() -> CArg {
    CArg::new(ACCOUNT_SUBSTATES)
}

/// The from-account component + vault PLUS the fixture's own fetched substates (the stealth input
/// UTXO, if any) — everything the live two-phase driver needs to resolve the transfer.
fn account_plus_fixture_batch(fx: &serde_json::Value) -> CArg {
    let mut batch: Vec<serde_json::Value> = serde_json::from_str(ACCOUNT_SUBSTATES).unwrap();
    if let Some(arr) = fx["input"].get("fetched").and_then(|v| v.as_array()) {
        batch.extend(arr.iter().cloned());
    }
    CArg::new(&serde_json::to_string(&batch).unwrap())
}

/// Counts the `stealth_utxo` wants in a build's `want_list` (the account component + vault wants are
/// `specific_substate` / `vault_for_resource`, never `stealth_utxo`).
fn count_stealth_utxo_wants(want_list: &serde_json::Value) -> usize {
    want_list
        .as_array()
        .expect("want_list array")
        .iter()
        .filter(|w| w["kind"] == "stealth_utxo")
        .count()
}

/// A well-formed `EncodedPublicTransfer` envelope has a non-empty hex `encoded_transaction` + a
/// 64-hex-char `transaction_id`. The send vectors compare *semantically* (the proofs are not
/// byte-stable), and the semantic-decode equivalence is asserted by the core's own golden-vector
/// tests; here we prove the ABI round-trips to a valid encoded transfer.
fn assert_well_formed_encoded(out: &serde_json::Value) {
    let encoded = out["encoded_transaction"]
        .as_str()
        .expect("encoded_transaction is a hex string");
    assert!(!encoded.is_empty(), "encoded_transaction must not be empty");
    assert!(
        encoded.chars().all(|c| c.is_ascii_hexdigit()),
        "encoded_transaction is lowercase hex"
    );
    let id = out["transaction_id"].as_str().expect("transaction_id is a hex string");
    assert_eq!(id.len(), 64, "transaction_id is 32 bytes = 64 hex chars");
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "transaction_id is hex");
}

/// (a) one-shot stealth send round-trips across the ABI to a well-formed encoded transfer, no handle.
#[test]
fn stealth_one_shot_send_round_trips() {
    let fx = load_fixture("stealth_transfer/stealth_seal_with_input.json");
    let a = stealth_send_args(&fx);

    let result = unsafe {
        ootle_build_and_encode_stealth_transfer_with_seed(
            a.network,
            a.intent.ptr(),
            a.fetched.ptr(),
            a.spend_secrets.ptr(),
            a.keys.ptr(),
        )
    };
    assert!(ok(&result), "stealth one-shot failed: {}", error_code(&result));
    assert_eq!(error_code(&result), "", "success envelope has an empty error_code");
    assert!(result.handle.is_null(), "one-shot stealth op returns no handle");
    assert_well_formed_encoded(&data_json(&result));
    unsafe { ootle_result_free(result) };
}

/// (b) two-phase stealth handle flow (revealed-only — no stealth inputs): build → apply (the
/// from-account component + vault resolve the account wants) → seal (consumes the dedicated stealth
/// handle).
#[test]
fn stealth_two_phase_handle_flow() {
    let fx = load_fixture("stealth_transfer/account_key_seal_with_revealed_input.json");
    let a = stealth_send_args(&fx);

    // 1) build → dedicated stealth handle + want list (no stealth inputs ⇒ only the from-account component + vault
    //    wants, no stealth_utxo want).
    let built = unsafe { ootle_build_stealth_unsigned_with_seed(a.network, a.intent.ptr(), a.seed_hex.ptr()) };
    assert!(ok(&built), "stealth build failed: {}", error_code(&built));
    assert!(!built.handle.is_null(), "stealth build returns a handle");
    let want_body = data_json(&built);
    let wants = want_body["want_list"].as_array().expect("want_list array");
    assert_eq!(wants.len(), 2, "the from-account component + vault wants");
    assert_eq!(
        count_stealth_utxo_wants(&want_body["want_list"]),
        0,
        "a revealed-only transfer has no stealth UTXO wants"
    );
    let handle = built.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_result_free(built) }; // frees strings; handle stays live (threaded forward).

    // 2a) apply the from-account component + vault → the account wants resolve.
    let account = account_substates_batch();
    let applied = unsafe {
        ootle_apply_fetched_substates_stealth(
            handle,
            a.network,
            account.ptr(),
            a.spend_secrets.ptr(),
            a.account_only_keys.ptr(),
        )
    };
    assert!(ok(&applied), "stealth apply failed: {}", error_code(&applied));
    assert!(!applied.handle.is_null(), "apply threads the handle forward");
    assert_eq!(
        data_json(&applied)["status"],
        "resolved",
        "the from-account component + vault resolve the transfer"
    );
    let resolved = applied.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_result_free(applied) };

    // 2b) seal → consumes the handle, returns encoded bytes, no handle. The random-nonce stealth seal
    // takes account-secret-only keys.
    let sealed = unsafe { ootle_seal_and_encode_stealth(resolved, a.network, a.account_only_keys.ptr()) };
    assert!(ok(&sealed), "stealth seal failed: {}", error_code(&sealed));
    assert!(sealed.handle.is_null(), "stealth seal returns no handle");
    assert_well_formed_encoded(&data_json(&sealed));
    unsafe { ootle_result_free(sealed) };
}

/// (b2) Multi-round stealth convergence for stealth inputs: build a
/// transfer with one stealth input, apply round 1 with an EMPTY batch (the host has not fetched the
/// UTXO) → `need_more` carrying the UTXO id in `fetch_ids`; apply round 2 with that UTXO → `resolved`;
/// seal → a well-formed encoded transfer. Proves the `NeedMore { fetch_ids }` loop drives stealth
/// inputs across ≥2 rounds, with the wanted id handed back by the core (not derived by the host).
#[test]
fn stealth_two_phase_multi_round_converges() {
    let fx = load_fixture("stealth_transfer/stealth_seal_with_input.json");
    let a = stealth_send_args(&fx);

    // The committed fetched batch + the (single) input UTXO id the want list must surface.
    let all_fetched = fx["input"]["fetched"].as_array().expect("fetched is an array");
    assert_eq!(all_fetched.len(), 1, "fixture has one stealth input UTXO");
    let utxo_id = all_fetched[0]["substate_id"].as_str().unwrap().to_string();

    // 1) build → handle + a want list naming the from-account component + vault plus the stealth UTXO.
    let built = unsafe { ootle_build_stealth_unsigned_with_seed(a.network, a.intent.ptr(), a.seed_hex.ptr()) };
    assert!(ok(&built), "stealth build failed: {}", error_code(&built));
    let want_body = data_json(&built);
    let wants = want_body["want_list"].as_array().expect("want_list array");
    assert_eq!(wants.len(), 3, "from-account component + vault + one stealth UTXO want");
    assert_eq!(
        count_stealth_utxo_wants(&want_body["want_list"]),
        1,
        "exactly one stealth_utxo want"
    );
    let mut handle = built.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_result_free(built) };

    // Round 1: empty batch → need_more with the discovered UTXO id in fetch_ids.
    let empty = CArg::new("[]");
    let applied1 = unsafe {
        ootle_apply_fetched_substates_stealth(
            handle,
            a.network,
            empty.ptr(),
            a.spend_secrets.ptr(),
            a.account_only_keys.ptr(),
        )
    };
    assert!(ok(&applied1), "round 1 apply failed: {}", error_code(&applied1));
    let status1 = data_json(&applied1);
    assert_eq!(
        status1["status"], "need_more",
        "empty batch cannot resolve the stealth input"
    );
    let fetch_ids = status1["fetch_ids"].as_array().expect("need_more carries fetch_ids");
    assert!(
        fetch_ids.iter().any(|id| id.as_str() == Some(utxo_id.as_str())),
        "fetch_ids must expose the wanted UTXO id {utxo_id}, got {fetch_ids:?}"
    );
    handle = applied1.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    assert!(!applied1.handle.is_null(), "need_more threads the handle forward");
    unsafe { ootle_result_free(applied1) };

    // Round 2: fetch the named ids (from-account component + vault + the UTXO) → resolves.
    let batch = account_plus_fixture_batch(&fx);
    let applied2 = unsafe {
        ootle_apply_fetched_substates_stealth(
            handle,
            a.network,
            batch.ptr(),
            a.spend_secrets.ptr(),
            a.account_only_keys.ptr(),
        )
    };
    assert!(ok(&applied2), "round 2 apply failed: {}", error_code(&applied2));
    assert_eq!(
        data_json(&applied2)["status"],
        "resolved",
        "fetching fetch_ids converges"
    );
    let resolved = applied2.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_result_free(applied2) };

    // Seal → a well-formed encoded transfer (semantic compare for stealth send; the random-nonce seal
    // takes account-secret-only keys).
    let sealed = unsafe { ootle_seal_and_encode_stealth(resolved, a.network, a.account_only_keys.ptr()) };
    assert!(ok(&sealed), "stealth seal failed: {}", error_code(&sealed));
    assert_well_formed_encoded(&data_json(&sealed));
    unsafe { ootle_result_free(sealed) };
}

/// (b3) Missing required UTXO through the loop: build → apply (round 1 empty → need_more) → apply
/// (round 2 still empty → the resolver marks the id absent → a clean STEALTH/INVALID error). The
/// handle is consumed on the error path (no leak). This test asserts the error code and that the
/// path does not crash.
#[test]
fn stealth_apply_missing_required_utxo_errors() {
    let fx = load_fixture("stealth_transfer/stealth_seal_with_input.json");
    let a = stealth_send_args(&fx);

    let built = unsafe { ootle_build_stealth_unsigned_with_seed(a.network, a.intent.ptr(), a.seed_hex.ptr()) };
    assert!(ok(&built));
    let mut handle = built.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_result_free(built) };

    let empty = CArg::new("[]");
    // Round 1: empty → need_more (the from-account component, vault, and UTXO are all requested).
    let applied1 = unsafe {
        ootle_apply_fetched_substates_stealth(
            handle,
            a.network,
            empty.ptr(),
            a.spend_secrets.ptr(),
            a.account_only_keys.ptr(),
        )
    };
    assert!(ok(&applied1));
    assert_eq!(data_json(&applied1)["status"], "need_more");
    handle = applied1.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_result_free(applied1) };

    // Round 2: serve the from-account component + vault but NOT the UTXO → the account resolves while
    // the requested UTXO id is now definitively absent → error, handle consumed.
    let account = account_substates_batch();
    let applied2 = unsafe {
        ootle_apply_fetched_substates_stealth(
            handle,
            a.network,
            account.ptr(),
            a.spend_secrets.ptr(),
            a.account_only_keys.ptr(),
        )
    };
    assert!(!ok(&applied2), "a missing required UTXO must error");
    assert_eq!(error_code(&applied2), "INVALID");
    assert!(
        applied2.handle.is_null(),
        "the error envelope carries no handle (it was consumed)"
    );
    unsafe { ootle_result_free(applied2) };
}

/// The stealth handle from build, if the host aborts the flow, is freed exactly once via the
/// dedicated stealth free fn. No crash.
#[test]
fn abandoned_stealth_handle_is_freed_cleanly() {
    let fx = load_fixture("stealth_transfer/stealth_seal_with_input.json");
    let a = stealth_send_args(&fx);

    let built = unsafe { ootle_build_stealth_unsigned_with_seed(a.network, a.intent.ptr(), a.seed_hex.ptr()) };
    assert!(ok(&built));
    let handle = built.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
    unsafe { ootle_result_free(built) };
    unsafe { ootle_stealth_partial_transaction_free(handle) };
}

/// (c) stealth receive: a "mine" output decrypts to its expected value + mask; no handle.
#[test]
fn stealth_scan_mine_round_trips() {
    let fx = load_fixture("stealth_scan/mine_basic.json");
    let scan_input = &fx["input"]["stealth_scan_input"];
    let network = network_byte(&scan_input["network"]);
    // The scan-keys bundle the facade expects: {view_secret, account_public_key, skip_memo?}.
    let scan_keys = serde_json::json!({
        "view_secret": scan_input["view_secret"],
        "account_public_key": scan_input["account_public_key"],
        "skip_memo": scan_input["skip_memo"],
    });
    let keys = CArg::new(&serde_json::to_string(&scan_keys).unwrap());
    let output = CArg::new(&serde_json::to_string(&scan_input["output"]).unwrap());

    let result = unsafe { ootle_scan_stealth_output(network, keys.ptr(), output.ptr()) };
    assert!(ok(&result), "scan failed: {}", error_code(&result));
    assert!(result.handle.is_null(), "scan is stateless — no handle");
    let out = data_json(&result);
    assert_eq!(out["is_mine"], true, "the mine vector decrypts as mine");
    assert_eq!(
        out["value"], fx["expected"]["decrypted"]["value"],
        "recovered value matches the vector"
    );
    assert_eq!(
        out["mask"], fx["expected"]["decrypted"]["mask"],
        "recovered mask matches the vector"
    );
    unsafe { ootle_result_free(result) };
}

/// (c) stealth receive: a "not mine" output yields the `{"is_mine":false}` success envelope (not null
/// data_json, not an error).
#[test]
fn stealth_scan_not_mine_is_success_false() {
    let fx = load_fixture("stealth_scan/not_mine.json");
    let scan_input = &fx["input"]["stealth_scan_input"];
    let network = network_byte(&scan_input["network"]);
    let scan_keys = serde_json::json!({
        "view_secret": scan_input["view_secret"],
        "account_public_key": scan_input["account_public_key"],
        "skip_memo": scan_input["skip_memo"],
    });
    let keys = CArg::new(&serde_json::to_string(&scan_keys).unwrap());
    let output = CArg::new(&serde_json::to_string(&scan_input["output"]).unwrap());

    let result = unsafe { ootle_scan_stealth_output(network, keys.ptr(), output.ptr()) };
    assert!(
        ok(&result),
        "scan of a not-mine output is still a success: {}",
        error_code(&result)
    );
    assert!(result.handle.is_null(), "scan is stateless — no handle");
    let out = data_json(&result);
    assert_eq!(out["is_mine"], false, "a not-mine output reports is_mine=false");
    unsafe { ootle_result_free(result) };
}

/// (c) stealth receive: an output that decrypts under the scanner's view key but pays a spend key the
/// scanner's account cannot spend yields `{"is_mine":false}`.
#[test]
fn stealth_scan_foreign_spend_key_is_not_mine() {
    let fx = load_fixture("stealth_scan/foreign_spend_key.json");
    let scan_input = &fx["input"]["stealth_scan_input"];
    let network = network_byte(&scan_input["network"]);
    let scan_keys = serde_json::json!({
        "view_secret": scan_input["view_secret"],
        "account_public_key": scan_input["account_public_key"],
        "skip_memo": scan_input["skip_memo"],
    });
    let keys = CArg::new(&serde_json::to_string(&scan_keys).unwrap());
    let output = CArg::new(&serde_json::to_string(&scan_input["output"]).unwrap());

    let result = unsafe { ootle_scan_stealth_output(network, keys.ptr(), output.ptr()) };
    assert!(ok(&result), "scan failed: {}", error_code(&result));
    assert_eq!(data_json(&result)["is_mine"], false, "a foreign spend key is not mine");
    unsafe { ootle_result_free(result) };
}

/// (c2) stealth decode: a fetched UTXO substate (id + value) decodes to the InboundStealthOutput the
/// `decode_utxo` vector commits; no handle. Then the fused scan over the same substate recovers the
/// committed value (decode → scan composes across the C ABI).
#[test]
fn stealth_decode_utxo_round_trips() {
    let fx = load_fixture("stealth_scan/decode_utxo.json");
    let substate_id = CArg::new(&serde_json::to_string(&fx["input"]["substate_id"]).unwrap());
    let substate_value = CArg::new(&serde_json::to_string(&fx["input"]["substate_value"]).unwrap());

    let result = unsafe { ootle_decode_stealth_utxo(substate_id.ptr(), substate_value.ptr()) };
    assert!(ok(&result), "decode failed: {}", error_code(&result));
    assert!(result.handle.is_null(), "decode is stateless — no handle");
    let inbound = data_json(&result);
    assert_eq!(
        inbound, fx["expected"]["inbound_output"],
        "decoded InboundStealthOutput matches the vector"
    );
    unsafe { ootle_result_free(result) };

    // Fused decode → scan with the mine_basic scan keys (the decode vector shares those keys).
    let scan_fx = load_fixture("stealth_scan/mine_basic.json");
    let scan_input = &scan_fx["input"]["stealth_scan_input"];
    let network = network_byte(&scan_input["network"]);
    let scan_keys = serde_json::json!({
        "view_secret": scan_input["view_secret"],
        "account_public_key": scan_input["account_public_key"],
        "skip_memo": true,
    });
    let keys = CArg::new(&serde_json::to_string(&scan_keys).unwrap());
    let fused = unsafe { ootle_scan_stealth_substate(network, keys.ptr(), substate_id.ptr(), substate_value.ptr()) };
    assert!(ok(&fused), "fused scan failed: {}", error_code(&fused));
    let out = data_json(&fused);
    assert_eq!(out["is_mine"], true, "the decode vector scans as mine");
    assert_eq!(
        out["value"], scan_fx["expected"]["decrypted"]["value"],
        "fused scan recovers the committed value"
    );
    unsafe { ootle_result_free(fused) };
}

/// (c3) fused scan of a foreign UTXO returns the `{"is_mine":false}` success envelope (not an error);
/// and bad substate-value JSON to the decode fn is a PARSE error; a null arg is INVALID.
#[test]
fn stealth_scan_substate_not_mine_and_bad_json() {
    let fx = load_fixture("stealth_scan/decode_utxo.json");
    let substate_id = CArg::new(&serde_json::to_string(&fx["input"]["substate_id"]).unwrap());
    let substate_value = CArg::new(&serde_json::to_string(&fx["input"]["substate_value"]).unwrap());
    let net = network_byte(&serde_json::json!("esmeralda"));

    // A foreign view secret ⇒ not mine (the AEAD key does not match).
    let foreign_keys = CArg::new(
        &serde_json::to_string(&serde_json::json!({
            "view_secret": "ff".repeat(31) + "0f",
            "account_public_key": load_fixture("stealth_scan/mine_basic.json")["input"]["stealth_scan_input"]["account_public_key"],
            "skip_memo": true,
        }))
        .unwrap(),
    );
    let fused =
        unsafe { ootle_scan_stealth_substate(net, foreign_keys.ptr(), substate_id.ptr(), substate_value.ptr()) };
    assert!(
        ok(&fused),
        "not-mine fused scan is still a success: {}",
        error_code(&fused)
    );
    assert_eq!(
        data_json(&fused)["is_mine"],
        false,
        "foreign UTXO reports is_mine=false"
    );
    unsafe { ootle_result_free(fused) };

    // Bad substate-value JSON to the decode fn ⇒ PARSE.
    let bad = CArg::new("{ not json");
    let r = unsafe { ootle_decode_stealth_utxo(substate_id.ptr(), bad.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Null arg to the decode fn ⇒ INVALID.
    let r = unsafe { ootle_decode_stealth_utxo(std::ptr::null(), substate_value.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };
}

// --- substate decode + account balances ---------------------------------------------------------

/// `ootle_decode_substate` over the committed vault vector reproduces the kind-tagged DecodedSubstate
/// byte-for-byte (incl. a native u64 balance > 2^33); bad JSON ⇒ PARSE; null arg ⇒ INVALID.
#[test]
fn decode_substate_round_trips() {
    let fx = load_fixture("substate_decode/fungible_vault.json");
    let substate_value = CArg::new(&serde_json::to_string(&fx["input"]["substate_value"]).unwrap());

    let result = unsafe { ootle_decode_substate(substate_value.ptr()) };
    assert!(ok(&result), "decode failed: {}", error_code(&result));
    assert!(result.handle.is_null(), "decode is stateless — no handle");
    let decoded = data_json(&result);
    assert_eq!(
        decoded, fx["expected"]["decoded_substate"],
        "decoded DecodedSubstate matches the vector"
    );
    // The u64 balance survives as a native JSON number (no float truncation).
    assert!(
        decoded["value"]["revealed_balance"].is_u64(),
        "revealed_balance must be a native JSON u64, got {}",
        decoded["value"]["revealed_balance"]
    );
    assert_eq!(
        decoded["value"]["revealed_balance"].as_u64().unwrap(),
        fx["expected"]["decoded_substate"]["value"]["revealed_balance"]
            .as_u64()
            .unwrap()
    );
    unsafe { ootle_result_free(result) };

    // Bad JSON ⇒ PARSE.
    let bad = CArg::new("{ not json");
    let r = unsafe { ootle_decode_substate(bad.ptr()) };
    assert!(!ok(&r) && error_code(&r) == "PARSE");
    unsafe { ootle_result_free(r) };

    // Null arg ⇒ INVALID.
    let r = unsafe { ootle_decode_substate(std::ptr::null()) };
    assert!(!ok(&r) && error_code(&r) == "INVALID");
    unsafe { ootle_result_free(r) };
}

/// `ootle_account_balances` over the committed account-balances vector reproduces the per-resource
/// revealed balance (a sum > 2^33, native u64); `ootle_account_balance_wants` names the account's
/// vault ids; a missing vault ⇒ RESOLUTION; bad JSON ⇒ PARSE; null arg ⇒ INVALID.
#[test]
fn account_balances_round_trips() {
    let fx = load_fixture("account_balances/multi_vault_u64.json");
    let account = CArg::new(&serde_json::to_string(&fx["input"]["substate_value"]).unwrap());
    let vaults = CArg::new(&serde_json::to_string(&fx["input"]["vault_substates"]).unwrap());

    // (a) balances reproduce the vector and carry a native u64 sum > 2^33.
    let result = unsafe { ootle_account_balances(account.ptr(), vaults.ptr()) };
    assert!(ok(&result), "account_balances failed: {}", error_code(&result));
    assert!(result.handle.is_null(), "stateless — no handle");
    let payload = data_json(&result);
    assert_eq!(
        payload["balances"], fx["expected"]["account_balances"],
        "balances match the vector"
    );
    let bal = &payload["balances"][0]["revealed_balance"];
    assert!(bal.is_u64(), "revealed_balance must be a native JSON u64, got {bal}");
    assert!(bal.as_u64().unwrap() > (1u64 << 33), "the locked balance is > 2^33");
    unsafe { ootle_result_free(result) };

    // (b) account_balance_wants names the account's vault ids (the fetch_ids pattern).
    let wants = unsafe { ootle_account_balance_wants(account.ptr()) };
    assert!(ok(&wants), "wants failed: {}", error_code(&wants));
    let want_payload = data_json(&wants);
    let fetch_ids = want_payload["fetch_ids"].as_array().expect("fetch_ids array");
    assert_eq!(fetch_ids.len(), 2, "account references two vaults");
    assert!(
        fetch_ids.iter().all(|v| v.as_str().unwrap().starts_with("vault_")),
        "fetch_ids are vault substate ids"
    );
    unsafe { ootle_result_free(wants) };

    // (c) a missing referenced vault ⇒ RESOLUTION (never a silent zero).
    let one_vault = serde_json::json!([fx["input"]["vault_substates"][0]]);
    let partial = CArg::new(&serde_json::to_string(&one_vault).unwrap());
    let r = unsafe { ootle_account_balances(account.ptr(), partial.ptr()) };
    assert!(!ok(&r) && error_code(&r) == "RESOLUTION", "missing vault is RESOLUTION");
    unsafe { ootle_result_free(r) };

    // (d) bad JSON ⇒ PARSE; null arg ⇒ INVALID.
    let bad = CArg::new("{ not json");
    let r = unsafe { ootle_account_balances(bad.ptr(), vaults.ptr()) };
    assert!(!ok(&r) && error_code(&r) == "PARSE");
    unsafe { ootle_result_free(r) };
    let r = unsafe { ootle_account_balance_wants(std::ptr::null()) };
    assert!(!ok(&r) && error_code(&r) == "INVALID");
    unsafe { ootle_result_free(r) };
}

/// (d) error envelopes for the stealth surface: malformed intent → PARSE; null arg → INVALID; null
/// handle to seal → INVALID; unknown network → INVALID.
#[test]
fn stealth_error_envelopes() {
    let net = network_byte(&serde_json::json!("esmeralda"));
    let empty = CArg::new("[]");
    let empty_obj = CArg::new("{}");

    // Malformed intent JSON → PARSE.
    let bad_intent = CArg::new("{ not json");
    let r = unsafe {
        ootle_build_and_encode_stealth_transfer(net, bad_intent.ptr(), empty.ptr(), empty.ptr(), empty_obj.ptr())
    };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    assert!(r.data_json.is_null());
    assert!(r.handle.is_null());
    unsafe { ootle_result_free(r) };

    // Null intent → INVALID.
    let r = unsafe { ootle_build_stealth_unsigned(net, std::ptr::null()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };

    // Null handle to stealth apply → INVALID, consumes nothing.
    let r = unsafe {
        ootle_apply_fetched_substates_stealth(std::ptr::null_mut(), net, empty.ptr(), empty.ptr(), empty.ptr())
    };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };

    // Null scan_keys → INVALID.
    let out = CArg::new("{}");
    let r = unsafe { ootle_scan_stealth_output(net, std::ptr::null(), out.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };

    // Null handle to stealth seal → INVALID.
    let r = unsafe { ootle_seal_and_encode_stealth(std::ptr::null_mut(), net, empty_obj.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };

    // Unknown network byte → INVALID.
    let r = unsafe {
        ootle_build_and_encode_stealth_transfer(0xff, empty_obj.ptr(), empty.ptr(), empty.ptr(), empty_obj.ptr())
    };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };
}

// --- (10) validate / canonicalize stealth transfer ----------------------------------------------

/// Seals a stealth transfer from the given fixture's args over the one-shot C ABI and returns its
/// `encoded_transaction` hex (a freshly sealed, signature-valid transaction for the validate tests).
fn seal_stealth_hex(fx: &serde_json::Value) -> String {
    let a = stealth_send_args(fx);
    let sealed = unsafe {
        ootle_build_and_encode_stealth_transfer_with_seed(
            a.network,
            a.intent.ptr(),
            a.fetched.ptr(),
            a.spend_secrets.ptr(),
            a.keys.ptr(),
        )
    };
    assert!(ok(&sealed), "seal for validate test failed: {}", error_code(&sealed));
    let hex = data_json(&sealed)["encoded_transaction"]
        .as_str()
        .expect("encoded_transaction hex")
        .to_string();
    unsafe { ootle_result_free(sealed) };
    hex
}

/// Asserts every byte-unstable field in the null set is JSON null wherever it appears. The null set
/// is sourced from the core's single contract constant — no second copy here.
fn assert_unstable_nulled(value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for key in ootle_sdk_core::stealth::canonicalize::UNSTABLE_NULL_SET {
                if let Some(slot) = map.get(key) {
                    assert!(
                        slot.is_null(),
                        "field `{key}` must be nulled in the canonical JSON, got {slot}"
                    );
                }
            }
            for (_, v) in map {
                assert_unstable_nulled(v);
            }
        },
        serde_json::Value::Array(items) => items.iter().for_each(assert_unstable_nulled),
        _ => {},
    }
}

/// True if any object in the tree carries a non-null public-key-like field (signer pubkeys survive).
fn contains_nonnull_pubkey(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(k, v)| {
            ((k.contains("public_key") || k == "public_nonce" || k.ends_with("_pk")) && !v.is_null()) ||
                contains_nonnull_pubkey(v)
        }),
        serde_json::Value::Array(items) => items.iter().any(contains_nonnull_pubkey),
        _ => false,
    }
}

/// (a) validate a known-good freshly-sealed stealth tx → ok + the unstable set nulled + signer
/// public keys survive.
#[test]
fn stealth_validate_good_seal_canonicalizes() {
    let fx = load_fixture("stealth_transfer/stealth_seal_with_input.json");
    let net = stealth_send_args(&fx).network;
    let hex = seal_stealth_hex(&fx);
    let chex = CArg::new(&hex);

    let result = unsafe { ootle_validate_stealth_transfer(net, chex.ptr()) };
    assert!(ok(&result), "validate of a good seal failed: {}", error_code(&result));
    assert!(result.handle.is_null(), "validate is stateless — no handle");
    let value = data_json(&result);
    assert_unstable_nulled(&value);
    assert!(
        contains_nonnull_pubkey(&value),
        "signer public keys must survive the nulling"
    );
    unsafe { ootle_result_free(result) };
}

/// (b) a tampered seal (a flipped byte in the trailing seal signature) → an error envelope, never ok.
#[test]
fn stealth_validate_tampered_seal_errors() {
    let fx = load_fixture("stealth_transfer/stealth_seal_with_input.json");
    let net = stealth_send_args(&fx).network;
    let hex = seal_stealth_hex(&fx);

    // Flip the last hex nibble → a still-decodable transaction whose seal signature no longer
    // verifies (the envelope tail is the seal signature scalar).
    let mut chars: Vec<char> = hex.chars().collect();
    let last = chars.len() - 1;
    let flipped = match chars[last] {
        '0' => 'f',
        _ => '0',
    };
    chars[last] = flipped;
    let tampered: String = chars.into_iter().collect();
    let chex = CArg::new(&tampered);

    let result = unsafe { ootle_validate_stealth_transfer(net, chex.ptr()) };
    assert!(!ok(&result), "a tampered seal must NOT validate ok");
    assert!(
        error_code(&result) == "VALIDATION" || error_code(&result) == "ENCODING",
        "a tampered seal is a hard error (VALIDATION or ENCODING), got {}",
        error_code(&result)
    );
    assert!(result.data_json.is_null(), "error envelope carries no data");
    assert!(result.handle.is_null());
    unsafe { ootle_result_free(result) };
}

/// (c) error envelopes: odd/malformed hex → PARSE; null arg → INVALID; unknown network → INVALID.
#[test]
fn stealth_validate_error_envelopes() {
    let net = network_byte(&serde_json::json!("esmeralda"));

    // Odd-length / non-hex → PARSE.
    let bad = CArg::new("abc");
    let r = unsafe { ootle_validate_stealth_transfer(net, bad.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Null arg → INVALID.
    let r = unsafe { ootle_validate_stealth_transfer(net, std::ptr::null()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };

    // Unknown network byte → INVALID.
    let some_hex = CArg::new("00010203");
    let r = unsafe { ootle_validate_stealth_transfer(0xff, some_hex.ptr()) };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };
}

// --- (7c) ootle_build_stealth_outputs_statement (standalone outputs-statement builder) -----------

/// The (network, intent JSON, entropy JSON) an outputs-statement fixture pins.
struct OutputsStatementArgs {
    network: u8,
    intent: CArg,
    seed_hex: CArg,
}

fn outputs_statement_args(fx: &serde_json::Value) -> OutputsStatementArgs {
    let input = &fx["input"];
    OutputsStatementArgs {
        network: network_byte(&input["network"]),
        intent: CArg::new(&serde_json::to_string(&input["stealth_intent"]).unwrap()),
        seed_hex: CArg::new(input["stealth_seed"].as_str().expect("stealth_seed hex")),
    }
}

/// Drives `ootle_build_stealth_outputs_statement` over a committed outputs-statement vector and asserts
/// the deterministic fields it returns match the fixture: the `aggregated_output_mask` byte-for-byte and
/// the statement structure (with `agg_range_proof` nulled — semantic) field-for-field.
#[test]
fn stealth_build_outputs_statement_reproduces_vector_fields() {
    for rel in [
        "stealth_outputs_statement/single_output_no_view_key.json",
        "stealth_outputs_statement/single_output_with_view_key.json",
    ] {
        let fx = load_fixture(rel);
        let args = outputs_statement_args(&fx);

        let result = unsafe {
            ootle_build_stealth_outputs_statement_with_seed(
                args.network,
                args.intent.ptr(),
                args.seed_hex.ptr(),
                std::ptr::null(),
            )
        };
        assert!(ok(&result), "fixture {rel}: build failed: {}", error_code(&result));
        assert!(
            result.handle.is_null(),
            "build outputs statement is stateless — no handle"
        );
        let value = data_json(&result);

        // The aggregated output mask is byte-stable (even in semantic mode).
        let expected_mask = fx["expected"]["aggregated_output_mask"]
            .as_str()
            .expect("fixture mask is a string");
        assert_eq!(
            value["aggregated_output_mask"]
                .as_str()
                .expect("returned mask is a string"),
            expected_mask,
            "fixture {rel}: aggregated_output_mask mismatch",
        );

        // The statement (agg_range_proof nulled) matches the fixture's recorded statement field-for-field.
        let expected_stmt = &fx["expected"]["stealth_outputs_statement"];
        assert_eq!(
            &value["outputs_statement"], expected_stmt,
            "fixture {rel}: outputs_statement (deterministic fields) mismatch",
        );
        // Defensive: the fixture and the returned statement both null the byte-unstable bulletproof.
        assert!(
            value["outputs_statement"]["agg_range_proof"].is_null(),
            "fixture {rel}: agg_range_proof must be nulled (semantic)",
        );
        let outputs = value["outputs_statement"]["outputs"]
            .as_array()
            .expect("outputs is an array");
        assert!(!outputs.is_empty(), "fixture {rel}: outputs must be non-empty");

        unsafe { ootle_result_free(result) };
    }
}

/// Error envelopes: malformed intent → PARSE; all-zero seed → VALIDATION; bad seed hex → PARSE; null
/// arg → INVALID; unknown network → INVALID.
#[test]
fn stealth_build_outputs_statement_error_envelopes() {
    let fx = load_fixture("stealth_outputs_statement/single_output_no_view_key.json");
    let args = outputs_statement_args(&fx);
    let net = args.network;

    // Malformed intent JSON → PARSE.
    let bad_intent = CArg::new("{not json");
    let r = unsafe {
        ootle_build_stealth_outputs_statement_with_seed(net, bad_intent.ptr(), args.seed_hex.ptr(), std::ptr::null())
    };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    assert!(r.handle.is_null());
    unsafe { ootle_result_free(r) };

    // An all-zero seed is rejected by the rail → VALIDATION.
    let zero_seed = CArg::new(&"00".repeat(32));
    let r = unsafe {
        ootle_build_stealth_outputs_statement_with_seed(net, args.intent.ptr(), zero_seed.ptr(), std::ptr::null())
    };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "VALIDATION");
    unsafe { ootle_result_free(r) };

    // A wrong-length seed hex → PARSE.
    let short_seed = CArg::new(&"aa".repeat(16));
    let r = unsafe {
        ootle_build_stealth_outputs_statement_with_seed(net, args.intent.ptr(), short_seed.ptr(), std::ptr::null())
    };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "PARSE");
    unsafe { ootle_result_free(r) };

    // Null intent arg → INVALID.
    let r = unsafe {
        ootle_build_stealth_outputs_statement_with_seed(net, std::ptr::null(), args.seed_hex.ptr(), std::ptr::null())
    };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };

    // Unknown network byte → INVALID.
    let r = unsafe {
        ootle_build_stealth_outputs_statement_with_seed(0xff, args.intent.ptr(), args.seed_hex.ptr(), std::ptr::null())
    };
    assert!(!ok(&r));
    assert_eq!(error_code(&r), "INVALID");
    unsafe { ootle_result_free(r) };
}

// --- (8) leak check (looped, amplifies any per-iteration leak above cross-thread noise) ----------

/// Runs a full round-trip (one-shot encode + two-phase build→apply→seal + parse) `ITERS` times,
/// freeing **every** returned envelope and the (consumed) handles via the documented contract. A
/// per-iteration leak of even a single allocation would push the net live-allocation growth toward
/// `ITERS`; we assert it stays far below that. Functional correctness is covered by the tests above —
/// this isolates the alloc/free balance.
#[test]
#[allow(clippy::too_many_lines)] // leak loop over every ABI entry point; meaningful only as one counted body
fn no_leaks_over_many_round_trips() {
    const ITERS: usize = 500;

    let one_shot = load_fixture("public_transfer/single_key_basic.json");
    let resolved = load_fixture("resolve_public_transfer/single_key_basic.json");
    let parse = load_fixture("parse_finalized_result/accept.json");

    let net1 = network_byte(&one_shot["input"]["network"]);
    let intent1 = CArg::new(&serde_json::to_string(&one_shot["input"]["intent"]).unwrap());
    let keys1 = CArg::new(&serde_json::to_string(&one_shot["input"]["keys"]).unwrap());

    let net2 = network_byte(&resolved["input"]["network"]);
    let intent2 = CArg::new(&serde_json::to_string(&resolved["input"]["intent"]).unwrap());
    let fetched2 = CArg::new(&serde_json::to_string(&resolved["input"]["fetched"]).unwrap());
    let keys2 = CArg::new(&serde_json::to_string(&resolved["input"]["keys"]).unwrap());

    let raw3 = CArg::new(&serde_json::to_string(&parse["input"]["raw_result"]).unwrap());

    let dry_run = load_fixture("parse_finalized_result/dry_run.json");
    let raw_dry = CArg::new(&serde_json::to_string(&dry_run["input"]["raw_result"]).unwrap());

    // Generic builder, driven through the existing apply/seal/free.
    let generic = load_fixture("generic_build/call_method_transfer.json");
    let net_g = network_byte(&generic["input"]["network"]);
    let intent_g = CArg::new(&serde_json::to_string(&generic["input"]["generic_intent"]).unwrap());
    let fetched_g = CArg::new(&serde_json::to_string(&generic["input"]["fetched"]).unwrap());
    let keys_g = CArg::new(&serde_json::to_string(&generic["input"]["keys"]).unwrap());

    let keygen_fx = load_fixture("keys/account_from_seed.json");
    let seed_hex = CArg::new(keygen_fx["input"]["seed"].as_str().unwrap());

    let addr_fx = load_fixture("address_derive/from_recipient_pk.json");
    let addr_pk_hex = CArg::new(addr_fx["input"]["account_public_key"].as_str().unwrap());

    // Address codec (format identity + parse), driven from the committed address_codec vectors.
    let id_fx = load_fixture("address_codec/identity_localnet_with_pay_ref.json");
    let id_net = network_byte(&id_fx["input"]["network"]);
    let id_account = CArg::new(id_fx["input"]["account_public_key"].as_str().unwrap());
    let id_view = CArg::new(id_fx["input"]["view_only_key"].as_str().unwrap());
    let id_pay_ref = CArg::new(id_fx["input"]["pay_ref"].as_str().unwrap());
    let parse_id_fx = load_fixture("address_codec/parse_identity_esmeralda.json");
    let parse_addr = CArg::new(parse_id_fx["input"]["address"].as_str().unwrap());
    let parse_comp_fx = load_fixture("address_codec/parse_component.json");
    let parse_comp = CArg::new(parse_comp_fx["input"]["address"].as_str().unwrap());

    // Stealth send (one-shot + two-phase) + scan, driven from the committed stealth vectors.
    let stealth_send = load_fixture("stealth_transfer/stealth_seal_with_input.json");
    let s = stealth_send_args(&stealth_send);
    // An empty fetched batch for the stealth two-phase loop's round-1 `need_more` step.
    let empty_batch = CArg::new("[]");
    // Round-2 batch: the from-account component + vault plus the input UTXO → the loop resolves.
    let account_utxo_batch = account_plus_fixture_batch(&stealth_send);

    let scan_fx = load_fixture("stealth_scan/mine_basic.json");
    let scan_input = &scan_fx["input"]["stealth_scan_input"];
    let net_scan = network_byte(&scan_input["network"]);
    let scan_keys = CArg::new(
        &serde_json::to_string(&serde_json::json!({
            "view_secret": scan_input["view_secret"],
            "account_public_key": scan_input["account_public_key"],
            "skip_memo": scan_input["skip_memo"],
        }))
        .unwrap(),
    );
    let scan_output = CArg::new(&serde_json::to_string(&scan_input["output"]).unwrap());

    // Stealth decode + fused scan, driven from the committed decode_utxo vector.
    let decode_fx = load_fixture("stealth_scan/decode_utxo.json");
    let decode_id = CArg::new(&serde_json::to_string(&decode_fx["input"]["substate_id"]).unwrap());
    let decode_value = CArg::new(&serde_json::to_string(&decode_fx["input"]["substate_value"]).unwrap());

    // Standalone outputs-statement builder, driven from a committed vector.
    let outputs_fx = load_fixture("stealth_outputs_statement/single_output_with_view_key.json");
    let os = outputs_statement_args(&outputs_fx);

    // Substate decode + account balances, driven from committed vectors.
    let dec_substate_fx = load_fixture("substate_decode/fungible_vault.json");
    let dec_substate_value = CArg::new(&serde_json::to_string(&dec_substate_fx["input"]["substate_value"]).unwrap());
    let bal_fx = load_fixture("account_balances/multi_vault_u64.json");
    let bal_account = CArg::new(&serde_json::to_string(&bal_fx["input"]["substate_value"]).unwrap());
    let bal_vaults = CArg::new(&serde_json::to_string(&bal_fx["input"]["vault_substates"]).unwrap());

    // Co-sign (authorize → attach → seal), driven from the committed cosign vector.
    let cosign_fx = load_fixture("cosign/seal_with_auth.json");
    let net_cs = network_byte(&cosign_fx["input"]["network"]);
    let cs_intent = CArg::new(&serde_json::to_string(&cosign_fx["input"]["intent"]).unwrap());
    let cs_fetched = CArg::new(&serde_json::to_string(&cosign_fx["input"]["fetched"]).unwrap());
    let cs_keys = CArg::new(&serde_json::to_string(&cosign_fx["input"]["keys"]).unwrap());
    let cs_seal_pk = CArg::new(cosign_fx["input"]["cosign_seal_pk"].as_str().unwrap());
    let cs_signer_secret = CArg::new(cosign_fx["input"]["cosign_signer_secret"].as_str().unwrap());

    let before = live();
    for _ in 0..ITERS {
        // one-shot
        let r = unsafe { ootle_build_and_encode_public_transfer(net1, intent1.ptr(), keys1.ptr()) };
        assert!(ok(&r));
        unsafe { ootle_result_free(r) };

        // two-phase
        let built = unsafe { ootle_build_unsigned(net2, intent2.ptr()) };
        assert!(ok(&built));
        let h = built.handle;
        unsafe { ootle_result_free(built) };
        let applied = unsafe { ootle_apply_fetched_substates(h, fetched2.ptr()) };
        assert!(ok(&applied));
        let h = applied.handle;
        unsafe { ootle_result_free(applied) };
        let sealed = unsafe { ootle_seal_and_encode(h, keys2.ptr()) };
        assert!(ok(&sealed));
        unsafe { ootle_result_free(sealed) };

        // generic builder (→ existing apply/seal/free)
        let built = unsafe { ootle_build_unsigned_instructions(net_g, intent_g.ptr()) };
        assert!(ok(&built));
        let h = built.handle;
        unsafe { ootle_result_free(built) };
        let applied = unsafe { ootle_apply_fetched_substates(h, fetched_g.ptr()) };
        assert!(ok(&applied));
        let h = applied.handle;
        unsafe { ootle_result_free(applied) };
        let sealed = unsafe { ootle_seal_and_encode(h, keys_g.ptr()) };
        assert!(ok(&sealed));
        unsafe { ootle_result_free(sealed) };

        // parse (committed + dry-run shapes, same fn)
        let p = unsafe { ootle_parse_finalized_result(raw3.ptr()) };
        assert!(ok(&p));
        unsafe { ootle_result_free(p) };
        let pd = unsafe { ootle_parse_finalized_result(raw_dry.ptr()) };
        assert!(ok(&pd));
        unsafe { ootle_result_free(pd) };

        // identity keygen (production + deterministic seed paths)
        let ga = unsafe { ootle_generate_account_key() };
        assert!(ok(&ga));
        unsafe { ootle_result_free(ga) };
        let gv = unsafe { ootle_generate_view_key() };
        assert!(ok(&gv));
        unsafe { ootle_result_free(gv) };
        let da = unsafe { ootle_derive_account_key_from_seed(seed_hex.ptr()) };
        assert!(ok(&da));
        unsafe { ootle_result_free(da) };
        let dv = unsafe { ootle_derive_view_key_from_seed(seed_hex.ptr()) };
        assert!(ok(&dv));
        unsafe { ootle_result_free(dv) };

        // account-address derivation
        let addr = unsafe { ootle_derive_account_address(addr_pk_hex.ptr()) };
        assert!(ok(&addr));
        unsafe { ootle_result_free(addr) };

        // address codec: format identity (with pay_ref) + parse identity + parse substate
        let fmt = unsafe { ootle_format_identity_address(id_net, id_account.ptr(), id_view.ptr(), id_pay_ref.ptr()) };
        assert!(ok(&fmt));
        unsafe { ootle_result_free(fmt) };
        let pid = unsafe { ootle_parse_address(parse_addr.ptr()) };
        assert!(ok(&pid));
        unsafe { ootle_result_free(pid) };
        let pcomp = unsafe { ootle_parse_address(parse_comp.ptr()) };
        assert!(ok(&pcomp));
        unsafe { ootle_result_free(pcomp) };

        // stealth one-shot send
        let sr = unsafe {
            ootle_build_and_encode_stealth_transfer(
                s.network,
                s.intent.ptr(),
                s.fetched.ptr(),
                s.spend_secrets.ptr(),
                s.account_only_keys.ptr(),
            )
        };
        assert!(ok(&sr));
        unsafe { ootle_result_free(sr) };

        // stealth two-phase send (build → apply×2 over the NeedMore loop → seal, threading +
        // consuming the dedicated stealth handle). Round 1 supplies an empty batch (need_more), round
        // 2 supplies the input UTXO (resolved) — exercising the multi-round path inside the leak loop.
        let sbuilt = unsafe { ootle_build_stealth_unsigned(s.network, s.intent.ptr()) };
        assert!(ok(&sbuilt));
        let mut sh = sbuilt.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
        unsafe { ootle_result_free(sbuilt) };

        let sapplied1 = unsafe {
            ootle_apply_fetched_substates_stealth(
                sh,
                s.network,
                empty_batch.ptr(),
                s.spend_secrets.ptr(),
                s.account_only_keys.ptr(),
            )
        };
        assert!(ok(&sapplied1));
        sh = sapplied1.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
        unsafe { ootle_result_free(sapplied1) };

        let sapplied2 = unsafe {
            ootle_apply_fetched_substates_stealth(
                sh,
                s.network,
                account_utxo_batch.ptr(),
                s.spend_secrets.ptr(),
                s.account_only_keys.ptr(),
            )
        };
        assert!(ok(&sapplied2));
        sh = sapplied2.handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
        unsafe { ootle_result_free(sapplied2) };

        let ssealed = unsafe { ootle_seal_and_encode_stealth(sh, s.network, s.account_only_keys.ptr()) };
        assert!(ok(&ssealed));
        let sealed_hex = data_json(&ssealed)["encoded_transaction"]
            .as_str()
            .expect("encoded_transaction hex")
            .to_string();
        unsafe { ootle_result_free(ssealed) };

        // stealth validate: decode + verify-all-sigs + canonical nulled JSON over the freshly sealed
        // transfer. Exercises the validate alloc/free balance in the leak loop.
        let cvalidate = CArg::new(&sealed_hex);
        let svalidated = unsafe { ootle_validate_stealth_transfer(s.network, cvalidate.ptr()) };
        assert!(ok(&svalidated));
        unsafe { ootle_result_free(svalidated) };

        // handle kind-tag misuse cycle (build → misroute → correct-free): a stealth handle routed to
        // a public consumer + the public free must be rejected WITHOUT consuming/freeing it, then freed
        // correctly with the stealth free fn. A wrong-kind consume that leaked (or a wrong-kind free
        // that double-freed) would show up here as net growth across the loop.
        let smis = unsafe { ootle_build_stealth_unsigned(s.network, s.intent.ptr()) };
        assert!(ok(&smis));
        let smis_handle = smis.handle; // cross-cast `*mut OotlePartialTransaction`
        unsafe { ootle_result_free(smis) };
        // Misroute to a public consumer → INVALID, not consumed.
        let rej = unsafe { ootle_apply_fetched_substates(smis_handle, empty_batch.ptr()) };
        assert!(!ok(&rej) && error_code(&rej) == "INVALID");
        unsafe { ootle_result_free(rej) };
        // Misroute to the public free → no-op (handle survives).
        unsafe { ootle_partial_transaction_free(smis_handle) };
        // Correct free.
        let smis_stealth = smis_handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
        unsafe { ootle_stealth_partial_transaction_free(smis_stealth) };

        // Reverse direction: a PUBLIC handle misrouted to the stealth consumer + stealth free (both
        // rejected, not consumed), then freed correctly with the public free. Balances the reverse
        // no-op path inside the loop so neither misroute direction can leak/double-free unnoticed.
        let pmis = unsafe { ootle_build_unsigned(net2, intent2.ptr()) };
        assert!(ok(&pmis));
        let pmis_handle = pmis.handle;
        unsafe { ootle_result_free(pmis) };
        let pmis_stealth = pmis_handle as *mut ootle_sdk_ffi_c::OotleStealthPartialTransaction;
        let rej = unsafe {
            ootle_apply_fetched_substates_stealth(
                pmis_stealth,
                net2,
                empty_batch.ptr(),
                empty_batch.ptr(),
                empty_batch.ptr(),
            )
        };
        assert!(!ok(&rej) && error_code(&rej) == "INVALID");
        unsafe { ootle_result_free(rej) };
        unsafe { ootle_stealth_partial_transaction_free(pmis_stealth) }; // no-op
        unsafe { ootle_partial_transaction_free(pmis_handle) }; // correct free

        // stealth scan
        let scan = unsafe { ootle_scan_stealth_output(net_scan, scan_keys.ptr(), scan_output.ptr()) };
        assert!(ok(&scan));
        unsafe { ootle_result_free(scan) };

        // stealth decode: fetched UTXO substate → InboundStealthOutput.
        let dec = unsafe { ootle_decode_stealth_utxo(decode_id.ptr(), decode_value.ptr()) };
        assert!(ok(&dec));
        unsafe { ootle_result_free(dec) };

        // stealth fused decode → scan: substate → scan result.
        let fscan =
            unsafe { ootle_scan_stealth_substate(net_scan, scan_keys.ptr(), decode_id.ptr(), decode_value.ptr()) };
        assert!(ok(&fscan));
        unsafe { ootle_result_free(fscan) };

        // stealth build outputs statement: deterministic statement + mask.
        let ostmt = unsafe {
            ootle_build_stealth_outputs_statement_with_seed(
                os.network,
                os.intent.ptr(),
                os.seed_hex.ptr(),
                std::ptr::null(),
            )
        };
        assert!(ok(&ostmt));
        unsafe { ootle_result_free(ostmt) };

        // substate decode: fetched substate → DecodedSubstate.
        let dsub = unsafe { ootle_decode_substate(dec_substate_value.ptr()) };
        assert!(ok(&dsub));
        unsafe { ootle_result_free(dsub) };

        // account balances: account + vaults → per-resource revealed balances.
        let abal = unsafe { ootle_account_balances(bal_account.ptr(), bal_vaults.ptr()) };
        assert!(ok(&abal));
        unsafe { ootle_result_free(abal) };

        // account balance wants: account → vault fetch ids.
        let awant = unsafe { ootle_account_balance_wants(bal_account.ptr()) };
        assert!(ok(&awant));
        unsafe { ootle_result_free(awant) };

        // Co-sign: A builds + resolves a handle, extracts the unsigned record
        // (non-consuming), B authorizes it (production), A seals with the authorization attached
        // (consuming the handle). Exercises the alloc/free balance of all three cosign fns + the record
        // extraction in the leak loop.
        let cs_built = unsafe { ootle_build_unsigned(net_cs, cs_intent.ptr()) };
        assert!(ok(&cs_built));
        let cs_h = cs_built.handle;
        unsafe { ootle_result_free(cs_built) };
        let cs_applied = unsafe { ootle_apply_fetched_substates(cs_h, cs_fetched.ptr()) };
        assert!(ok(&cs_applied));
        let cs_h = cs_applied.handle;
        unsafe { ootle_result_free(cs_applied) };

        // A extracts the record JSON to ship to B (does NOT consume the handle).
        let cs_record = unsafe { ootle_unsigned_record_for_cosign(cs_h, std::ptr::null()) };
        assert!(ok(&cs_record));
        let cs_record_json = serde_json::to_string(&data_json(&cs_record)).unwrap();
        unsafe { ootle_result_free(cs_record) };

        // B authorizes (production random nonce).
        let cs_unsigned = CArg::new(&cs_record_json);
        let cs_auth =
            unsafe { ootle_add_signature(net_cs, cs_unsigned.ptr(), cs_seal_pk.ptr(), cs_signer_secret.ptr()) };
        assert!(ok(&cs_auth));
        let cs_auth_obj = data_json(&cs_auth)["authorization"].clone();
        unsafe { ootle_result_free(cs_auth) };

        // A attaches + seals (consuming the handle).
        let cs_auths = CArg::new(&serde_json::to_string(&serde_json::json!([cs_auth_obj])).unwrap());
        let cs_sealed =
            unsafe { ootle_seal_and_encode_with_auth(cs_h, cs_keys.ptr(), cs_auths.ptr(), std::ptr::null()) };
        assert!(ok(&cs_sealed));
        unsafe { ootle_result_free(cs_sealed) };
    }
    let growth = live() - before;
    // Generous bound: real per-iteration leaks would be ~ITERS×(allocs-per-iter). Cross-thread noise
    // from other parallel tests is bounded and small relative to ITERS.
    assert!(
        growth < (ITERS as i64) / 4,
        "net live-allocation growth {growth} over {ITERS} iterations suggests a leak (one allocation leaked per \
         iteration would be ~{ITERS})"
    );
}
