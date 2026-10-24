//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The flat `extern "C"` substate-decode + account-balances surface: read on-chain state back so a
//! host can verify an operation landed.
//!
//! Three stateless, handle-free fns mirroring the rest of the facade (same [`OotleResult`] envelope,
//! same [`guarded`]/[`flatten`] panic guard, same JSON-string marshalling, same stable error codes):
//!
//! - [`ootle_decode_substate`] — any fetched substate value (the indexer's base64 CBOR string) → the kind-tagged
//!   [`DecodedSubstate`](ootle_sdk_core::DecodedSubstate).
//! - [`ootle_account_balances`] — an account component + its fetched vault substates → the revealed balance per
//!   resource (never a confidential total; see the core docs).
//! - [`ootle_account_balance_wants`] — an account component → the `vault_<hex>` ids to fetch.
//!
//! Balances cross as native JSON `u64`, never stringified-and-reparsed through a float. The host does
//! the fetch; the core never touches the network.

use std::os::raw::c_char;

use ootle_sdk_core::{FetchedSubstate, account_balance_wants, account_balances, decode_substate};

use crate::c_abi::{OotleResult, flatten, guarded, output_json, parse_json, required_str};

/// Decodes any fetched substate value into the kind-tagged
/// [`DecodedSubstate`](ootle_sdk_core::DecodedSubstate). **Stateless — returns no handle.**
///
/// `substate_value_json` is the indexer's substate value (a JSON base64 CBOR string), passed verbatim. On success
/// `data_json` is `{ "kind": "component|vault|resource|other", "value": { … } }`. For a vault the
/// `value.revealed_balance` is the revealed/unlocked amount only — the confidential side is surfaced
/// as `value.confidential_commitment_count` + `value.kind`, never a silent zero. Balances are native
/// JSON `u64`.
///
/// A malformed/unknown substate value yields `"PARSE"`; a balance that does not fit a `u64` yields
/// `"VALIDATION"`; a null arg yields `"INVALID"`.
///
/// # Safety
/// `substate_value_json` must be a valid NUL-terminated UTF-8 C string. The returned envelope must be
/// freed with [`ootle_result_free`](crate::ootle_result_free). It never carries a handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_decode_substate(substate_value_json: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let substate_value_json = unsafe { required_str(substate_value_json, "substate_value_json") }?;
            let substate_value: serde_json::Value = parse_json(substate_value_json, "substate value")?;

            match decode_substate(&substate_value) {
                Ok(decoded) => Ok(OotleResult::ok_json(&output_json(&decoded, "decoded substate")?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Computes the **revealed** balance per resource for an account. **Stateless — returns no handle.**
///
/// `account_substate_json` is the account `Component` substate value as the indexer serves it; `vault_substates_json`
/// is a JSON array of [`FetchedSubstate`](ootle_sdk_core::FetchedSubstate) records (`{substate_id, version,
/// substate_value}`) — the vaults the host already fetched (the ids
/// [`ootle_account_balance_wants`] named). The core rediscovers the account's vault ids from its CBOR
/// state and matches them by id: a referenced vault not supplied is a `"RESOLUTION"` error, never a
/// silent zero.
///
/// On success `data_json` is `{ "balances": [ { "resource_address": "resource_<hex>",
/// "revealed_balance": <u64> } ] }`. Revealed balances are native JSON `u64` and are the
/// revealed/unlocked amounts only (a confidential total requires scanning the relevant stealth UTXOs
/// separately).
///
/// A non-component account / a non-vault entry yields `"VALIDATION"`; a missing referenced vault
/// yields `"RESOLUTION"`; a malformed substate JSON yields `"PARSE"`; a null arg yields `"INVALID"`.
///
/// # Safety
/// `account_substate_json` and `vault_substates_json` must each be a valid NUL-terminated UTF-8 C
/// string. The returned envelope must be freed with [`ootle_result_free`](crate::ootle_result_free).
/// It never carries a handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_account_balances(
    account_substate_json: *const c_char,
    vault_substates_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let account_substate_json = unsafe { required_str(account_substate_json, "account_substate_json") }?;
            let vault_substates_json = unsafe { required_str(vault_substates_json, "vault_substates_json") }?;

            let account_substate: serde_json::Value = parse_json(account_substate_json, "account substate")?;
            let vault_substates: Vec<FetchedSubstate> = parse_json(vault_substates_json, "vault substates")?;

            match account_balances(&account_substate, &vault_substates) {
                Ok(balances) => {
                    // Wrap in `{ "balances": [...] }` per the documented wire shape.
                    let payload = serde_json::json!({ "balances": balances });
                    Ok(OotleResult::ok_json(&output_json(&payload, "account balances")?))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Names the `vault_<hex>` ids a host should fetch to satisfy [`ootle_account_balances`] for an
/// account. **Stateless — returns no handle.**
///
/// `account_substate_json` is the account `Component` substate value as the indexer serves it. On success `data_json`
/// is `{ "fetch_ids": [ "vault_<hex>", … ] }` — the same component-vault discovery
/// [`ootle_account_balances`] does, surfaced as opaque ids. The host fetches them and feeds them back
/// as `vault_substates_json`.
///
/// A non-component account yields `"VALIDATION"`; a malformed substate JSON yields `"PARSE"`; a null
/// arg yields `"INVALID"`.
///
/// # Safety
/// `account_substate_json` must be a valid NUL-terminated UTF-8 C string. The returned envelope must
/// be freed with [`ootle_result_free`](crate::ootle_result_free). It never carries a handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_account_balance_wants(account_substate_json: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let account_substate_json = unsafe { required_str(account_substate_json, "account_substate_json") }?;
            let account_substate: serde_json::Value = parse_json(account_substate_json, "account substate")?;

            match account_balance_wants(&account_substate) {
                Ok(fetch_ids) => {
                    let payload = serde_json::json!({ "fetch_ids": fetch_ids });
                    Ok(OotleResult::ok_json(&output_json(&payload, "account balance wants")?))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}
