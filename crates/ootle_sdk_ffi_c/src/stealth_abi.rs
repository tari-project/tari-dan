//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The flat `extern "C"` **stealth** (confidential-transfer) surface: send (two-phase + one-shot) and
//! receive (scan). It mirrors the public-transfer surface in [`crate::c_abi`] exactly — same envelope
//! ([`OotleResult`]), same panic guard ([`guarded`]/[`flatten`]), same error-code mapping
//! ([`OotleResult::from_core_err`]), same JSON-string marshalling — only the wrapped core entry points
//! and the handle type differ.
//!
//! ## The second opaque handle
//!
//! The stealth send flow threads its own opaque handle
//! [`OotleStealthPartialTransaction`] — a **distinct** type from the public-path
//! [`OotlePartialTransaction`](crate::OotlePartialTransaction) — with its own free fn
//! [`ootle_stealth_partial_transaction_free`]. It carries two internal shapes under one opaque type:
//! the in-progress two-phase **resolver** while inputs are being fetched, then the **assembled,
//! ready-to-seal** [`StealthPartialTransaction`] once resolution completes (see the handle's docs).
//! [`ootle_build_stealth_unsigned`] returns it; [`ootle_apply_fetched_substates_stealth`] **consumes
//! and re-issues** it each round; [`ootle_seal_and_encode_stealth`] **consumes** the assembled one
//! (taken by value via `Box::from_raw`); a host that aborts the flow frees it once with the free fn.
//!
//! ## The host-driven fetch loop (mirrors the public path)
//!
//! Stealth inputs resolve exactly like public inputs: a host-driven `NeedMore { fetch_ids }`
//! convergence loop with vault/UTXO discovery **in the core**.
//! [`ootle_build_stealth_unsigned`] returns the handle **plus** a want list (in `data_json`); the host
//! fetches the wanted substates and calls [`ootle_apply_fetched_substates_stealth`], which returns
//! either `{"status":"need_more","fetch_ids":[…]}` (fetch those concrete ids and repeat) or
//! `{"status":"resolved"}` (the handle now wraps the assembled partial — seal it). The host never
//! derives substate ids or supplies the fetched inputs up front; the core hands back exactly what to
//! fetch. (The one-shot [`ootle_build_and_encode_stealth_transfer`] keeps taking everything up front —
//! it has no host to loop with.)
//!
//! ## Sensitive material is transient
//!
//! The account secret, the build seed, and the per-input spend secrets cross as lowercase-hex JSON
//! strings, are parsed into transient scalars for the duration of the call, and are dropped when it
//! returns. The facade does **not** zero them (it cannot reach into the host's `const char*`); treat
//! the host-owned argument buffers as the host's to manage.

use std::os::raw::c_char;

use ootle_sdk_core::{
    FetchedSubstate,
    RistrettoPublicKeyBytes,
    StealthKeys,
    StealthPartialTransaction,
    StealthResolution,
    apply_fetched_substates_stealth,
    build_and_encode_stealth_transfer,
    build_and_encode_stealth_transfer_with_seed,
    build_stealth_unsigned_with_wants,
    build_stealth_unsigned_with_wants_with_seed,
    decode_and_canonicalize_sealed_transfer,
    decode_stealth_utxo,
    scan_stealth_output,
    scan_stealth_substate,
    seal_and_encode_stealth_transfer,
    stealth::build_stealth_outputs_statement_with_seed,
    types::{
        bytes::{BuildSeed, PublicKeyBytes, SecretKeyBytes},
        stealth::{InboundStealthOutput, StealthTransferIntent},
    },
};
use serde::Deserialize;

use crate::c_abi::{
    HandleKind,
    OotleResult,
    flatten,
    guarded,
    handle_kind,
    network_from_byte,
    optional_str,
    output_json,
    parse_json,
    require_kind,
    required_str,
    seed_from_hex,
};

/// The opaque handle for the stealth send flow.
///
/// It threads two internal shapes under **one** opaque type (cbindgen emits an opaque forward
/// declaration; the host never inspects it):
///
/// - [`Resolving`](StealthHandleState::Resolving) — the in-progress two-phase resolver
///   ([`PartialTransaction`](ootle_sdk_core::PartialTransaction)) returned by [`ootle_build_stealth_unsigned`] and
///   threaded across [`ootle_apply_fetched_substates_stealth`] rounds (`NeedMore`); and
/// - [`Ready`](StealthHandleState::Ready) — the assembled, input-resolved [`StealthPartialTransaction`] the final apply
///   produces (`Resolved`), which [`ootle_seal_and_encode_stealth`] consumes.
///
/// Keeping both under the same opaque type means the kind tag only has to distinguish
/// stealth-vs-public, not resolver-vs-ready. The handle is freed once with
/// [`ootle_stealth_partial_transaction_free`] if the host abandons the flow.
///
/// **Type-confusion guard:** the shared [`OotleResult::handle`] field is typed as the public
/// `*mut OotlePartialTransaction`; this handle reaches the host through a cross-cast. To make
/// misrouting a **deterministic error rather than UB**, this struct is `#[repr(C)]` with a
/// [`HandleKind`](crate::c_abi::HandleKind)`::Stealth` as its **first** field (matching the public
/// handle's layout prefix). Every stealth consumer/free reads the kind through the shared header
/// before taking ownership and rejects a public handle with an `INVALID` error; symmetrically the
/// public consumers reject this one. See [`HandleKind`](crate::c_abi::HandleKind).
///
/// cbindgen never emits this struct's body (it is `cbindgen:no-export`); the header declares it as an
/// **opaque forward declaration** (injected via `cbindgen.toml`'s `after_includes`), so the host only
/// ever sees an opaque pointer and the wire contract is unchanged.
///
/// cbindgen:no-export
#[repr(C)]
pub struct OotleStealthPartialTransaction {
    pub(crate) kind: HandleKind,
    pub(crate) inner: StealthHandleState,
}

/// The two internal shapes the stealth handle carries across its lifecycle (resolver → ready). See
/// [`OotleStealthPartialTransaction`].
///
/// cbindgen:no-export
pub(crate) enum StealthHandleState {
    /// The in-progress two-phase resolver (build → apply* rounds).
    Resolving(Box<ootle_sdk_core::PartialTransaction>),
    /// The assembled, ready-to-seal product (after the final apply resolves).
    Ready(Box<StealthPartialTransaction>),
}

// --- Facade-local JSON mirrors --------------------------------------------------------------------

/// Wire mirror of the core's [`StealthKeys`] (which carries secret newtypes and does **not** derive
/// `Deserialize`). All fields lowercase hex. `seed` is the **construction** seed (per-output
/// commitments/AEAD/proof entropy) the seed-reproducible build path consumes — the signatures are
/// signed later with fresh random nonces, so a seed never feeds a Schnorr nonce.
#[derive(Debug, Deserialize)]
struct StealthKeysJson {
    account_secret: SecretKeyBytes,
    view_secret: SecretKeyBytes,
    seed: BuildSeed,
}

impl StealthKeysJson {
    /// The construction seed the keys carry (per-output commitment/AEAD/proof entropy).
    fn seed(&self) -> BuildSeed {
        self.seed
    }

    fn into_core(self) -> StealthKeys {
        StealthKeys::new(self.account_secret, self.view_secret)
    }
}

/// Random-path keys mirror: only the account and view secrets are needed — the construction entropy
/// is drawn from a fresh OS-RNG seed (see [`seal_and_encode_stealth_transfer`]).
#[derive(Debug, Deserialize)]
struct StealthProductionKeysJson {
    account_secret: SecretKeyBytes,
    view_secret: SecretKeyBytes,
}

impl StealthProductionKeysJson {
    fn into_core(self) -> StealthKeys {
        StealthKeys::new(self.account_secret, self.view_secret)
    }
}

/// Wire mirror of the scan-side argument bundle for [`scan_stealth_output`]. `view_secret` and
/// `account_public_key` are required: together they confirm the tag and the one-time spend key.
/// `skip_memo` defaults to `false`.
#[derive(Debug, Deserialize)]
struct StealthScanKeysJson {
    view_secret: SecretKeyBytes,
    account_public_key: PublicKeyBytes,
    #[serde(default)]
    skip_memo: bool,
}

/// Parses the positional per-input spend secrets (a JSON array of lowercase-hex secret scalars).
fn parse_spend_secrets(json: &str) -> Result<Vec<SecretKeyBytes>, OotleResult> {
    parse_json(json, "spend secrets")
}

/// Extracts the assembled, ready-to-seal partial from a stealth handle, erroring (`STEALTH`) if the
/// handle is still an in-progress resolver (the host must drive
/// [`ootle_apply_fetched_substates_stealth`] to `resolved` before sealing). The handle has already
/// been consumed by the caller, so a still-resolving handle is a precondition violation, not a leak.
fn ready_or_err(state: StealthHandleState) -> Result<StealthPartialTransaction, OotleResult> {
    match state {
        StealthHandleState::Ready(p) => Ok(*p),
        StealthHandleState::Resolving(_) => Err(OotleResult::err(
            "STEALTH",
            "stealth handle is not yet resolved — drive ootle_apply_fetched_substates_stealth to \"resolved\" before \
             sealing",
        )),
    }
}

// --- One-shot send --------------------------------------------------------------------------------

/// One-shot random-nonce default stealth send: build (resolve inputs + assemble) and seal/encode in
/// one call, expanding all proof + seal entropy from a fresh OS-RNG seed.
///
/// Arguments (all `const char*` UTF-8 JSON unless noted):
/// - `network` — the L1 network discriminant byte.
/// - `intent_json` — a `StealthTransferIntent`.
/// - `fetched_json` — a JSON array of `FetchedSubstate`: **every** stealth-input UTXO substate, fetched up front (the
///   wanted `utxo_<…>` ids are a pure function of each input's resource + commitment).
/// - `spend_secrets_json` — a JSON array of lowercase-hex secret scalars, one **positional** per `intent.inputs`, used
///   to decrypt each input's spend mask.
/// - `keys_json` — `{account_secret, view_secret}` (lowercase hex).
///
/// On success `data_json` is the `EncodedPublicTransfer` (`{encoded_transaction, transaction_id}`,
/// lowercase hex; the wire shape is shared with the public path). The bytes/id are **not** reproducible
/// — use [`ootle_build_and_encode_stealth_transfer_with_seed`] for the reproducible path.
///
/// # Safety
/// `intent_json`, `fetched_json`, `spend_secrets_json`, and `keys_json` must each be a valid
/// NUL-terminated UTF-8 C string. The returned envelope must be freed with
/// [`ootle_result_free`](crate::ootle_result_free). The secrets cross as transient hex JSON and are
/// dropped at the end of the call; the facade does not zero them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_build_and_encode_stealth_transfer(
    network: u8,
    intent_json: *const c_char,
    fetched_json: *const c_char,
    spend_secrets_json: *const c_char,
    keys_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let intent_json = unsafe { required_str(intent_json, "intent_json") }?;
            let fetched_json = unsafe { required_str(fetched_json, "fetched_json") }?;
            let spend_secrets_json = unsafe { required_str(spend_secrets_json, "spend_secrets_json") }?;
            let keys_json = unsafe { required_str(keys_json, "keys_json") }?;

            let intent: StealthTransferIntent = parse_json(intent_json, "intent")?;
            let fetched: Vec<FetchedSubstate> = parse_json(fetched_json, "fetched substates")?;
            let spend_secrets = parse_spend_secrets(spend_secrets_json)?;
            let keys: StealthProductionKeysJson = parse_json(keys_json, "keys")?;

            match build_and_encode_stealth_transfer(network, &intent, &fetched, &spend_secrets, &keys.into_core()) {
                Ok(encoded) => Ok(OotleResult::ok_json(&output_json(
                    &encoded,
                    "encoded stealth transfer",
                )?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Seed-reproducible counterpart of [`ootle_build_and_encode_stealth_transfer`]. The single build seed
/// in `keys_json` (`{account_secret, view_secret, seed}`, lowercase hex) expands into **both** the account-key
/// authorization + seal nonces and the proof + seal-side entropy, so the result is reproducible from
/// the seed + intent (except the aggregated bulletproof, which is never byte-stable — compare
/// *semantically*).
///
/// # Safety
/// As [`ootle_build_and_encode_stealth_transfer`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_build_and_encode_stealth_transfer_with_seed(
    network: u8,
    intent_json: *const c_char,
    fetched_json: *const c_char,
    spend_secrets_json: *const c_char,
    keys_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let intent_json = unsafe { required_str(intent_json, "intent_json") }?;
            let fetched_json = unsafe { required_str(fetched_json, "fetched_json") }?;
            let spend_secrets_json = unsafe { required_str(spend_secrets_json, "spend_secrets_json") }?;
            let keys_json = unsafe { required_str(keys_json, "keys_json") }?;

            let intent: StealthTransferIntent = parse_json(intent_json, "intent")?;
            let fetched: Vec<FetchedSubstate> = parse_json(fetched_json, "fetched substates")?;
            let spend_secrets = parse_spend_secrets(spend_secrets_json)?;
            let keys: StealthKeysJson = parse_json(keys_json, "keys")?;
            let seed = keys.seed();

            match build_and_encode_stealth_transfer_with_seed(
                network,
                &intent,
                &fetched,
                &spend_secrets,
                &keys.into_core(),
                &seed,
            ) {
                Ok(encoded) => Ok(OotleResult::ok_json(&output_json(
                    &encoded,
                    "encoded stealth transfer",
                )?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Two-phase send -------------------------------------------------------------------------------

/// Seeds the stealth input-resolution loop (random-nonce default: a fresh OS-RNG seed), returning the
/// opaque stealth handle (in `OotleResult.handle`) **plus** the want list in `data_json`
/// (`{"want_list":[…]}` — the serde form of `WantList`'s inner `Vec<WantItem>`).
///
/// Takes `intent_json` only — **no** up-front `fetched`/`spend_secrets` (the host drives the fetch
/// loop). The host fetches the wanted substates, then calls
/// [`ootle_apply_fetched_substates_stealth`] until it returns `{"status":"resolved"}`, at which point
/// the threaded handle wraps the assembled partial ready for [`ootle_seal_and_encode_stealth`]. The
/// expanded entropy is stashed in the handle so the final apply can assemble without re-passing it. The
/// eventual bytes are **not** reproducible — use [`ootle_build_stealth_unsigned_with_seed`] for the
/// reproducible path.
///
/// Pass the returned handle only to the `ootle_*_stealth*` consumers /
/// [`ootle_stealth_partial_transaction_free`]. Routing it to the public-path consumers / free
/// (`ootle_apply_fetched_substates` / `ootle_seal_and_encode` / `ootle_partial_transaction_free`)
/// returns an `INVALID` error and leaves the handle intact.
///
/// # Safety
/// `intent_json` must be a valid NUL-terminated UTF-8 C string. The returned envelope must be freed
/// with [`ootle_result_free`](crate::ootle_result_free); the returned `handle` must be consumed by
/// [`ootle_apply_fetched_substates_stealth`] (each round) and ultimately
/// [`ootle_seal_and_encode_stealth`], or freed with [`ootle_stealth_partial_transaction_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_build_stealth_unsigned(network: u8, intent_json: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let intent_json = unsafe { required_str(intent_json, "intent_json") }?;
            let intent: StealthTransferIntent = parse_json(intent_json, "intent")?;

            match build_stealth_unsigned_with_wants(network, &intent) {
                Ok((partial, want_list)) => {
                    let body = serde_json::json!({ "want_list": want_list.0 });
                    Ok(OotleResult::ok_stealth_handle_json(
                        &output_json(&body, "want list")?,
                        StealthHandleState::Resolving(Box::new(partial)),
                    ))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Seed-reproducible counterpart of [`ootle_build_stealth_unsigned`]. `seed_hex` is the lowercase-hex
/// 32-byte build seed expanded into the stashed proof + seal-side entropy, so the eventual sealed bytes
/// are reproducible from the seed (except the aggregated bulletproof). A bad/odd/uppercase/wrong-length
/// `seed_hex` ⇒ `"PARSE"`; an all-zero seed ⇒ `"VALIDATION"`.
///
/// # Safety
/// `intent_json` and `seed_hex` must each be a valid NUL-terminated UTF-8 C string. The returned
/// handle lifecycle is identical to [`ootle_build_stealth_unsigned`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_build_stealth_unsigned_with_seed(
    network: u8,
    intent_json: *const c_char,
    seed_hex: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let intent_json = unsafe { required_str(intent_json, "intent_json") }?;
            let seed_hex = unsafe { required_str(seed_hex, "seed_hex") }?;
            let intent: StealthTransferIntent = parse_json(intent_json, "intent")?;
            let seed = BuildSeed::from_array(seed_from_hex(seed_hex)?);

            match build_stealth_unsigned_with_wants_with_seed(network, &intent, &seed) {
                Ok((partial, want_list)) => {
                    let body = serde_json::json!({ "want_list": want_list.0 });
                    Ok(OotleResult::ok_stealth_handle_json(
                        &output_json(&body, "want list")?,
                        StealthHandleState::Resolving(Box::new(partial)),
                    ))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Applies a fetched batch of substates to the stealth resolver and reports resolution status.
///
/// **Consumes** `handle` (taken by value — treat the pointer you passed as invalid afterwards, even on
/// error), like [`ootle_apply_fetched_substates`](crate::ootle_apply_fetched_substates). `network`
/// must be the transfer's network (the handle does not carry it). `fetched_json` is a JSON array of
/// `FetchedSubstate`; `spend_secrets_json` a JSON array of lowercase-hex secret scalars (one positional
/// per `intent.inputs`). On success the envelope carries a (possibly new) handle to thread forward and
/// `data_json`:
/// - `{"status":"resolved"}` — the handle wraps the **assembled** partial; seal it via
///   [`ootle_seal_and_encode_stealth`]; or
/// - `{"status":"need_more","fetch_ids":[…]}` — fetch the substates named in **`fetch_ids`** (the authoritative
///   concrete next-fetch set, including ids the core discovered) and call this again on the returned handle.
///
/// On a processing error the input handle is still consumed and freed; the returned envelope carries no
/// handle. Passing a handle that is **already** resolved (a `Ready` state) is a `STEALTH` error (the
/// handle is consumed). (Passing a *null* handle yields an `"INVALID"` envelope and consumes nothing.)
///
/// # Safety
/// `handle` must be a non-null pointer previously returned by [`ootle_build_stealth_unsigned`] /
/// [`ootle_build_stealth_unsigned_with_seed`] / this fn and not yet consumed. `fetched_json`,
/// `spend_secrets_json` and `keys_json` (`{account_secret, view_secret}`, lowercase hex — assembly names the revealed
/// output's receiver, which is the key that will seal) must each be a valid NUL-terminated UTF-8 C string. The returned
/// envelope must be freed with [`ootle_result_free`](crate::ootle_result_free).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_apply_fetched_substates_stealth(
    handle: *mut OotleStealthPartialTransaction,
    network: u8,
    fetched_json: *const c_char,
    spend_secrets_json: *const c_char,
    keys_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        if handle.is_null() {
            return OotleResult::err("INVALID", "argument `handle` must not be null");
        }
        // Validate the kind tag BEFORE taking ownership: a misrouted (public) handle is rejected here
        // and never reconstructed as a `Box<OotleStealthPartialTransaction>`.
        if let Err(e) = unsafe { require_kind(handle, HandleKind::Stealth) } {
            return e;
        }
        // Take ownership up front: the handle is consumed on every path (success or error), matching
        // the core apply fn's by-value signature. The host must not free it again.
        let state = unsafe { Box::from_raw(handle) }.inner;

        flatten((|| {
            let network = network_from_byte(network)?;
            let fetched_json = unsafe { required_str(fetched_json, "fetched_json") }?;
            let spend_secrets_json = unsafe { required_str(spend_secrets_json, "spend_secrets_json") }?;
            let keys_json = unsafe { required_str(keys_json, "keys_json") }?;
            let fetched: Vec<FetchedSubstate> = parse_json(fetched_json, "fetched substates")?;
            let spend_secrets = parse_spend_secrets(spend_secrets_json)?;
            // Assembly names the revealed output's receiver, which is the key that will seal, so the account secret
            // is needed here and not only at seal time.
            let keys: StealthProductionKeysJson = parse_json(keys_json, "keys")?;
            let keys = keys.into_core();

            // The handle must still be a resolver; a `Ready` (already-assembled) handle has no more
            // inputs to apply — seal it instead.
            let partial = match state {
                StealthHandleState::Resolving(p) => *p,
                StealthHandleState::Ready(_) => {
                    return Ok(OotleResult::err(
                        "STEALTH",
                        "stealth handle is already resolved — seal it, do not apply more substates",
                    ));
                },
            };

            match apply_fetched_substates_stealth(partial, network, &fetched, &spend_secrets, &keys) {
                Ok(StealthResolution::Resolved(ready)) => {
                    let body = serde_json::json!({ "status": "resolved" });
                    Ok(OotleResult::ok_stealth_handle_json(
                        &output_json(&body, "resolution")?,
                        StealthHandleState::Ready(ready),
                    ))
                },
                Ok(StealthResolution::NeedMore { partial, fetch_ids }) => {
                    let body = serde_json::json!({
                        "status": "need_more",
                        "fetch_ids": fetch_ids,
                    });
                    Ok(OotleResult::ok_stealth_handle_json(
                        &output_json(&body, "resolution")?,
                        StealthHandleState::Resolving(partial),
                    ))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Random-nonce default seal + BOR-encode of a stealth partial from [`ootle_build_stealth_unsigned`].
///
/// **Consumes** `handle` (taken by value — treat the pointer you passed as invalid afterwards, even on
/// error). `network` must be the transfer's network (the partial does not carry it; different networks
/// yield different stealth keys). `keys_json` is `{account_secret, view_secret}`; the seal nonces are expanded from
/// a fresh OS-RNG seed. On success `data_json` is the `EncodedPublicTransfer`. No handle is returned.
/// The bytes/id are not reproducible (random-nonce seal).
///
/// On a processing error the input handle is still consumed and freed; the returned envelope carries no
/// handle. (Passing a *null* handle is a precondition violation that yields an `"INVALID"` envelope and
/// consumes nothing.)
///
/// # Safety
/// `handle` must be a non-null pointer previously returned by [`ootle_build_stealth_unsigned`] and not
/// yet consumed or freed. `keys_json` must be a valid NUL-terminated UTF-8 C string. The returned
/// envelope must be freed with [`ootle_result_free`](crate::ootle_result_free). Do **not** free
/// `handle` afterwards — it is consumed here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_seal_and_encode_stealth(
    handle: *mut OotleStealthPartialTransaction,
    network: u8,
    keys_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        if handle.is_null() {
            return OotleResult::err("INVALID", "argument `handle` must not be null");
        }
        if let Err(e) = unsafe { require_kind(handle, HandleKind::Stealth) } {
            return e;
        }
        let state = unsafe { Box::from_raw(handle) }.inner;

        flatten((|| {
            let network = network_from_byte(network)?;
            let keys_json = unsafe { required_str(keys_json, "keys_json") }?;
            let keys: StealthProductionKeysJson = parse_json(keys_json, "keys")?;

            let partial = ready_or_err(state)?;
            match seal_and_encode_stealth_transfer(network, partial, &keys.into_core()) {
                Ok(encoded) => Ok(OotleResult::ok_json(&output_json(
                    &encoded,
                    "encoded stealth transfer",
                )?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Receive / scan -------------------------------------------------------------------------------

/// Scans an inbound stealth UTXO with a view secret and decides whether it is addressed to the scanner.
/// **Stateless — returns no handle.**
///
/// - `network` — the L1 network discriminant byte.
/// - `scan_keys_json` — `{view_secret, account_public_key, skip_memo?}` (lowercase hex; `skip_memo` defaults to
///   `false`).
/// - `inbound_output_json` — an `InboundStealthOutput` (the on-the-wire shape of a created stealth output).
///
/// `data_json` on success is **always** a JSON object:
/// - addressed to the scanner: the `DecryptedOutput` (`{is_mine:true, value, mask, memo}`); or
/// - **not** addressed to the scanner: `{"is_mine":false}` — a *success* envelope, deliberately **not** a null
///   `data_json` (a null payload on `ok=1` would be ambiguous for a thin host). A decrypt failing is "not mine", never
///   an error.
///
/// # Safety
/// `scan_keys_json` and `inbound_output_json` must each be a valid NUL-terminated UTF-8 C string. The
/// returned envelope must be freed with [`ootle_result_free`](crate::ootle_result_free). It never
/// carries a handle — do **not** call [`ootle_stealth_partial_transaction_free`] on its result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_scan_stealth_output(
    network: u8,
    scan_keys_json: *const c_char,
    inbound_output_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let scan_keys_json = unsafe { required_str(scan_keys_json, "scan_keys_json") }?;
            let inbound_output_json = unsafe { required_str(inbound_output_json, "inbound_output_json") }?;

            let scan_keys: StealthScanKeysJson = parse_json(scan_keys_json, "scan keys")?;
            let output: InboundStealthOutput = parse_json(inbound_output_json, "stealth output")?;

            match scan_stealth_output(
                network,
                &scan_keys.view_secret,
                &scan_keys.account_public_key,
                &output,
                scan_keys.skip_memo,
            ) {
                // Addressed to the scanner — emit the decrypted output verbatim (`is_mine` is in it).
                Ok(Some(decrypted)) => Ok(OotleResult::ok_json(&output_json(&decrypted, "decrypted output")?)),
                // Not mine — a success envelope with an explicit `{"is_mine":false}` object (never a
                // null `data_json`).
                Ok(None) => Ok(OotleResult::ok_json("{\"is_mine\":false}")),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Decode / fused scan ---------------------------------------------------------------------------

/// Decodes a fetched UTXO substate (the shape the indexer returns) into the receive-shaped
/// [`InboundStealthOutput`] the scanner consumes. **Stateless — returns no handle.**
///
/// - `substate_id_json` — the UTXO's canonical address string (`utxo_<resource>_<commitment>`), carrying the on-chain
///   commitment + resource address the value body omits. A JSON string.
/// - `substate_value_json` — the `SubstateValue` JSON the indexer returned, verbatim (the same shape the resolver
///   already consumes).
///
/// On success `data_json` is the decoded [`InboundStealthOutput`] (a JSON object with hex byte
/// fields). A non-UTXO / frozen / burnt substate or a malformed nonce yields `"INVALID"` / `"KEY"`;
/// a malformed substate id or undecodable value JSON yields `"PARSE"`; a null arg yields `"INVALID"`.
///
/// # Safety
/// `substate_id_json` and `substate_value_json` must each be a valid NUL-terminated UTF-8 C string.
/// The returned envelope must be freed with [`ootle_result_free`](crate::ootle_result_free). It never
/// carries a handle — do **not** call [`ootle_stealth_partial_transaction_free`] on its result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_decode_stealth_utxo(
    substate_id_json: *const c_char,
    substate_value_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let substate_id_json = unsafe { required_str(substate_id_json, "substate_id_json") }?;
            let substate_value_json = unsafe { required_str(substate_value_json, "substate_value_json") }?;

            let substate_id: String = parse_json(substate_id_json, "substate id")?;
            let substate_value: serde_json::Value = parse_json(substate_value_json, "substate value")?;

            match decode_stealth_utxo(&substate_id, &substate_value) {
                Ok(inbound) => Ok(OotleResult::ok_json(&output_json(&inbound, "inbound stealth output")?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Fused **decode → scan**: decode a fetched UTXO substate and scan it with the caller's view keys in
/// one call. **Stateless — returns no handle.**
///
/// Like [`ootle_scan_stealth_output`], this always returns a success envelope, never a null
/// `data_json`: not-mine ⇒ `{"is_mine":false}`, not an error.
///
/// - `network` — the L1 network discriminant byte.
/// - `scan_keys_json` — `{view_secret, account_public_key, skip_memo?}` (the same bundle [`ootle_scan_stealth_output`]
///   takes; lowercase hex; `skip_memo` defaults to `false`).
/// - `substate_id_json` — the UTXO's canonical address string. A JSON string.
/// - `substate_value_json` — the `SubstateValue` JSON the indexer returned, verbatim.
///
/// `data_json` on success is **always** a JSON object: the `DecryptedOutput`
/// (`{is_mine:true, value, mask, memo}`) when addressed to the scanner, or `{"is_mine":false}` when
/// not (a *success* envelope, never null). A decrypt-miss is "not mine", never an error. A malformed
/// substate id / value yields `"PARSE"`; a non-UTXO / frozen / burnt substate yields `"INVALID"`; a
/// null arg or unknown network yields `"INVALID"`.
///
/// # Safety
/// `scan_keys_json`, `substate_id_json`, and `substate_value_json` must each be a valid
/// NUL-terminated UTF-8 C string. The returned envelope must be freed with
/// [`ootle_result_free`](crate::ootle_result_free). It never carries a handle — do **not** call
/// [`ootle_stealth_partial_transaction_free`] on its result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_scan_stealth_substate(
    network: u8,
    scan_keys_json: *const c_char,
    substate_id_json: *const c_char,
    substate_value_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let scan_keys_json = unsafe { required_str(scan_keys_json, "scan_keys_json") }?;
            let substate_id_json = unsafe { required_str(substate_id_json, "substate_id_json") }?;
            let substate_value_json = unsafe { required_str(substate_value_json, "substate_value_json") }?;

            let scan_keys: StealthScanKeysJson = parse_json(scan_keys_json, "scan keys")?;
            let substate_id: String = parse_json(substate_id_json, "substate id")?;
            let substate_value: serde_json::Value = parse_json(substate_value_json, "substate value")?;

            match scan_stealth_substate(
                network,
                &scan_keys.view_secret,
                &scan_keys.account_public_key,
                &substate_id,
                &substate_value,
                scan_keys.skip_memo,
            ) {
                // Addressed to the scanner — emit the decrypted output verbatim (`is_mine` is in it).
                Ok(Some(decrypted)) => Ok(OotleResult::ok_json(&output_json(&decrypted, "decrypted output")?)),
                // Not mine — a success envelope with an explicit `{"is_mine":false}` object.
                Ok(None) => Ok(OotleResult::ok_json("{\"is_mine\":false}")),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Validate / canonicalize ----------------------------------------------------------------------

/// Decodes a sealed stealth transfer, verifies **every** signature on it, and returns its canonical
/// deterministic-field JSON (the byte-unstable proof/signature scalars nulled). **Stateless — returns
/// no handle.**
///
/// - `network` — the L1 network discriminant byte.
/// - `sealed_encoded_transaction_hex` — the lowercase-hex `EncodedPublicTransfer::encoded_transaction` (the
///   `TransactionEnvelope` wire form) produced by a stealth seal op.
///
/// On success `data_json` is the decoded transaction as a JSON object with the byte-unstable set
/// `["agg_range_proof","balance_proof","signature"]` recursively nulled (signer **public keys
/// survive**). A host compares it field-for-field against the expected canonical transaction to make
/// an accept/reject decision.
///
/// A **bad signature is an error, not a falsy success**: a tampered or otherwise invalid seal yields a
/// `"VALIDATION"` error envelope; malformed/odd-length hex yields `"PARSE"`; undecodable bytes yield
/// `"ENCODING"`; a null/invalid arg or unknown network yields `"INVALID"`.
///
/// # Safety
/// `sealed_encoded_transaction_hex` must be a valid NUL-terminated UTF-8 C string. The returned
/// envelope must be freed with [`ootle_result_free`](crate::ootle_result_free). It never carries a
/// handle — do **not** call [`ootle_stealth_partial_transaction_free`] on its result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_validate_stealth_transfer(
    network: u8,
    sealed_encoded_transaction_hex: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let sealed_hex = unsafe { required_str(sealed_encoded_transaction_hex, "sealed_encoded_transaction_hex") }?;

            match decode_and_canonicalize_sealed_transfer(network, sealed_hex) {
                Ok(value) => Ok(OotleResult::ok_json(&output_json(
                    &value,
                    "canonical stealth transfer",
                )?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Build outputs statement ----------------------------------------------------------------------

/// Builds the aggregated stealth **outputs statement** for an intent's outputs seed-reproducibly (every
/// nonce/mask expanded from the supplied build seed), returning the statement JSON + the aggregated
/// output mask. **Stateless — returns no handle.**
///
/// - `network` — the L1 network discriminant byte.
/// - `intent_json` — a [`StealthTransferIntent`] carrying the `outputs` (and `revealed_output_amount`).
/// - `seed_hex` — the lowercase-hex 32-byte build seed; it is expanded into one per-output entropy slice per
///   `intent.outputs` entry.
///
/// On success `data_json` is a JSON object:
/// ```json
/// { "outputs_statement": { … }, "aggregated_output_mask": "<64-hex>" }
/// ```
/// **Semantic, not byte-stable:** the statement's aggregated bulletproof (`agg_range_proof`) is
/// byte-unstable across runs, so it is recursively **nulled** in `outputs_statement`; every other
/// statement field and the `aggregated_output_mask` (a 32-byte scalar) **are** reproducible from the seed.
///
/// Malformed `intent` JSON yields `"PARSE"`; a bad/odd/uppercase/wrong-length `seed_hex` yields
/// `"PARSE"`; an all-zero seed yields `"VALIDATION"`; a null arg or unknown network yields `"INVALID"`.
///
/// # Safety
/// `intent_json` and `seed_hex` must each be a valid NUL-terminated UTF-8 C string; `revealed_receiver_hex` is the
/// lowercase-hex public key authorised to take the intent's revealed output, and may be null only when the intent
/// reveals nothing. The returned
/// envelope must be freed with [`ootle_result_free`](crate::ootle_result_free). It never carries a
/// handle — do **not** call [`ootle_stealth_partial_transaction_free`] on its result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_build_stealth_outputs_statement_with_seed(
    network: u8,
    intent_json: *const c_char,
    seed_hex: *const c_char,
    revealed_receiver_hex: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let intent_json = unsafe { required_str(intent_json, "intent_json") }?;
            let seed_hex = unsafe { required_str(seed_hex, "seed_hex") }?;

            let intent: StealthTransferIntent = parse_json(intent_json, "intent")?;
            let seed = BuildSeed::from_array(seed_from_hex(seed_hex)?);
            let revealed_receiver = unsafe { optional_str(revealed_receiver_hex, "revealed_receiver_hex") }?
                .map(|hex| {
                    RistrettoPublicKeyBytes::from_hex(hex).map_err(|e| {
                        OotleResult::err("PARSE", &format!("revealed_receiver_hex is not a public key: {e}"))
                    })
                })
                .transpose()?;

            match build_stealth_outputs_statement_with_seed(network, &intent, &seed, revealed_receiver) {
                Ok((stmt, mask)) => {
                    // Serialize the statement, then null the byte-unstable aggregated range proof so
                    // the semantic compare is stable.
                    let mut stmt_value = serde_json::to_value(&stmt).map_err(|e| {
                        OotleResult::err("ENCODING", &format!("failed to serialize outputs statement: {e}"))
                    })?;
                    if let Some(obj) = stmt_value.as_object_mut() {
                        obj.insert("agg_range_proof".to_string(), serde_json::Value::Null);
                    }
                    let body = serde_json::json!({
                        "outputs_statement": stmt_value,
                        "aggregated_output_mask": mask.to_hex(),
                    });
                    Ok(OotleResult::ok_json(&output_json(&body, "stealth outputs statement")?))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Free fn --------------------------------------------------------------------------------------

/// Frees an opaque [`OotleStealthPartialTransaction`] handle. Null-safe; call **exactly once**, and
/// **only** for a handle that was never consumed by [`ootle_seal_and_encode_stealth`] (which takes the
/// handle by value). Freeing a consumed handle is a use-after-free.
///
/// **Kind-guarded:** if the handle is actually a public-path `OotlePartialTransaction` (misrouted), it
/// is **not** freed — the call is a deterministic no-op that leaves the handle intact for the correct
/// free fn ([`ootle_partial_transaction_free`](crate::ootle_partial_transaction_free)). This prevents a
/// bad free / type confusion.
///
/// # Safety
/// `handle` must be either null or an [`OotleStealthPartialTransaction`] pointer obtained from this
/// library and not yet consumed or freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_stealth_partial_transaction_free(handle: *mut OotleStealthPartialTransaction) {
    if handle.is_null() {
        return;
    }
    // Refuse a wrong-kind (public) handle: do NOT `Box::from_raw` it as a stealth handle (that would
    // be a bad free / type confusion). The handle is left intact for the correct free fn.
    if unsafe { handle_kind(handle) } != HandleKind::Stealth {
        return;
    }
    // SAFETY: pointer originated from `Box::into_raw` in this library, is the matching kind, and has
    // not been consumed.
    drop(unsafe { Box::from_raw(handle) });
}
