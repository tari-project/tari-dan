//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The flat `extern "C"` surface, the result envelope, the opaque handle, and the free functions.
//!
//! All `unsafe` and FFI mechanics are confined here. Every entry point follows the same shape: parse
//! the C inputs → call the pure core → marshal the outcome into an [`OotleResult`]. The whole body of
//! each entry point runs inside [`std::panic::catch_unwind`] so a panic can never unwind across the C
//! boundary (which is UB).

use std::{
    ffi::{CStr, CString},
    os::raw::c_char,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
};

use ootle_sdk_core::{
    Authorization,
    FaucetClaimIntent,
    FetchedSubstate,
    OotleKeypair,
    PartialTransaction,
    Resolution,
    UnsignedTransactionRecord,
    add_signature,
    apply_fetched_substates,
    build_and_encode_public_transfer,
    build_faucet_claim_with_wants,
    build_public_transfer_unsigned_with_wants,
    build_unsigned_instructions_with_wants,
    cosign::seal_and_encode_with_auth,
    derive_account_address,
    derive_account_keypair_from_seed,
    derive_view_keypair_from_seed,
    format_identity_address,
    generate_account_keypair,
    generate_view_keypair,
    keys::PublicTransferKeys,
    parse_address,
    parse_finalized_result,
    resolved_transfer::seal_and_encode_public_transfer,
    types::{
        bytes::{PublicKeyBytes, SecretKeyBytes},
        error::OotleSdkError,
        generic_intent::GenericTransactionIntent,
        intent::PublicTransferIntent,
        network::Network,
    },
};
use serde::Deserialize;

/// The stable ABI tag (as a NUL-terminated byte string so [`ootle_abi_version`] can hand out a
/// static pointer). Every host binding asserts this at startup to detect a header/lib mismatch. Bump on any
/// breaking ABI change (a changed signature, envelope layout, or handle contract) so a stale lib is
/// caught loudly rather than mis-marshalled.
const ABI_VERSION: &[u8] = b"ootle-sdk-ffi-c/19\0";

/// The kind discriminant that guards opaque-handle **type confusion** across the FFI.
///
/// The shared [`OotleResult::handle`] field is typed as `*mut OotlePartialTransaction` for **both**
/// the public path and the stealth path (the stealth handle is delivered via a cross-cast — see
/// [`OotleResult::ok_stealth_handle_json`]). Without a runtime discriminant, a host that routes a
/// stealth handle to a public consumer/free (or vice versa) would have its pointer reinterpreted as
/// the wrong opaque type → undefined behaviour (bad deref, bad free).
///
/// Both opaque handle structs ([`OotlePartialTransaction`] and
/// [`OotleStealthPartialTransaction`](crate::stealth_abi::OotleStealthPartialTransaction)) are
/// `#[repr(C)]` and carry a `kind: HandleKind` as their **first** field, so the tag sits at offset 0 of
/// the handle. A consumer reads the kind through a [`HandleHeader`]-typed view of the raw pointer
/// **before** taking ownership and rejects a mismatched handle with a deterministic `INVALID` error
/// instead of mis-`Box::from_raw`-ing it. This turns handle misrouting from UB into a clean, recoverable
/// error envelope.
///
/// Both handle structs are `cbindgen:no-export` (the host only threads an opaque pointer); the header
/// declares them as opaque forward declarations injected via `cbindgen.toml`'s `after_includes`, so the
/// added `kind` field never changes the C wire surface.
///
/// Crate-internal: this discriminant is an implementation detail of the guard, never part of the wire
/// surface.
///
/// cbindgen:no-export
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandleKind {
    /// A public-path [`OotlePartialTransaction`].
    Public = 0,
    /// A stealth-path
    /// [`OotleStealthPartialTransaction`](crate::stealth_abi::OotleStealthPartialTransaction).
    Stealth = 1,
}

/// The shared `#[repr(C)]` prefix of **every** opaque handle struct: a leading [`HandleKind`].
///
/// Because both handle structs put `kind` first with `#[repr(C)]`, casting any handle pointer to
/// `*const HandleHeader` and reading `.kind` is well-defined regardless of which concrete handle the
/// pointer actually addresses — the guard reads this **before** committing to a concrete
/// `Box::from_raw`. Internal only (cbindgen never emits it as a standalone type).
///
/// cbindgen:no-export
#[repr(C)]
pub(crate) struct HandleHeader {
    pub(crate) kind: HandleKind,
}

/// Reads the [`HandleKind`] of a non-null handle pointer through the shared [`HandleHeader`] prefix,
/// without taking ownership. The caller must have already null-checked `handle`.
///
/// # Safety
/// `handle` must be non-null and point to a struct produced by this library whose first field is a
/// `HandleKind` (i.e. an [`OotlePartialTransaction`] or
/// [`OotleStealthPartialTransaction`](crate::stealth_abi::OotleStealthPartialTransaction)). Every
/// boxed handle in this crate satisfies that invariant.
pub(crate) unsafe fn handle_kind<T>(handle: *const T) -> HandleKind {
    // SAFETY: both handle structs are `#[repr(C)]` with `kind: HandleKind` first, so the leading bytes
    // are a valid `HandleHeader` for either type — reading `kind` is well-defined even if the pointer's
    // static type is the *other* handle (the very misrouting this guards against). The `kind`-at-offset-0
    // invariant the cross-type read depends on is enforced at compile time by the assertions below.
    unsafe { (*(handle as *const HandleHeader)).kind }
}

// Compile-time guarantee that the cross-type kind-read is sound: `kind` must sit at offset 0 of BOTH
// opaque handle structs (and of `HandleHeader`), so reading it through a `*const HandleHeader` view of
// any handle pointer is well-defined regardless of which concrete handle the pointer addresses. These
// `#[repr(C)]` structs put `kind` first, so the offset is 0 today; the assertions fail the build if a
// future reorder/repr change ever breaks the invariant. (The handles are never passed across the C
// boundary by value — only as opaque pointers — so the non-FFI `inner` payloads don't affect this.)
const _: () = {
    assert!(std::mem::offset_of!(HandleHeader, kind) == 0);
    assert!(std::mem::offset_of!(OotlePartialTransaction, kind) == 0);
    assert!(std::mem::offset_of!(crate::stealth_abi::OotleStealthPartialTransaction, kind) == 0);
};

impl HandleKind {
    /// The human-readable name used in the kind-mismatch error message.
    fn label(self) -> &'static str {
        match self {
            HandleKind::Public => "public partial transaction",
            HandleKind::Stealth => "stealth partial transaction",
        }
    }
}

/// Validates that a non-null `handle` carries the `expected` [`HandleKind`] **before** any
/// `Box::from_raw`, returning an `INVALID` error envelope on mismatch (so a misrouted handle is never
/// reconstructed as the wrong `Box<T>`, never deref'd or freed as the wrong type). The caller must
/// have already null-checked `handle`.
///
/// # Safety
/// `handle` must be non-null and point to a handle struct produced by this library (first field a
/// `HandleKind`) — satisfied by every boxed handle in this crate.
pub(crate) unsafe fn require_kind<T>(handle: *const T, expected: HandleKind) -> Result<(), OotleResult> {
    // SAFETY: caller guarantees `handle` is a non-null library handle; see `handle_kind`.
    let actual = unsafe { handle_kind(handle) };
    if actual == expected {
        Ok(())
    } else {
        Err(OotleResult::err(
            "INVALID",
            &format!(
                "handle kind mismatch: expected {}, got {}",
                expected.label(),
                actual.label()
            ),
        ))
    }
}

/// The opaque handle type the host threads across the public-transfer calls.
///
/// It wraps a kind tag plus the core's [`PartialTransaction`]; the host only ever holds a pointer to it
/// and never inspects its fields. See the crate docs for the precise consume/free lifecycle.
///
/// `#[repr(C)]` with `kind` **first** guarantees the tag sits at offset 0, so a pointer to this handle
/// can be read through [`HandleHeader`] to get the [`HandleKind`] before any consumer/free takes
/// ownership. Routing a stealth handle here therefore fails deterministically (it never reaches
/// `Box::from_raw` as the wrong type).
///
/// cbindgen never emits this struct's body (it is `cbindgen:no-export`); the header declares the type
/// as an **opaque forward declaration** (injected via `cbindgen.toml`'s `after_includes`), so the host
/// only ever sees an opaque pointer and the wire contract is unchanged. See [`HandleKind`].
///
/// cbindgen:no-export
#[repr(C)]
pub struct OotlePartialTransaction {
    pub(crate) kind: HandleKind,
    pub(crate) inner: PartialTransaction,
}

/// The result envelope every `extern "C"` op returns by value.
///
/// `error_code`/`error_message`/`data_json` are heap-allocated, NUL-terminated UTF-8 C strings owned
/// by the **host**; free them all in one call with [`ootle_result_free`]. On success `error_code` and
/// `error_message` are the empty string `""` (never NULL); on error they carry the stable
/// [`OotleSdkError::code`] and the human-readable message, and the output fields are NULL/empty.
///
/// Exactly one of the output channels is populated per op: ops that return JSON set `data_json` (and
/// leave `handle` NULL); ops that return the opaque handle set `handle` (and leave `data_json` NULL).
/// [`ootle_result_free`] frees the three strings but **never** the handle — the handle has its own
/// lifecycle (see the crate docs).
#[repr(C)]
pub struct OotleResult {
    /// `1` on success, `0` on error. A plain `uint8_t` (not C `_Bool`) so the field has an identical,
    /// well-defined layout for every C / cgo host, including older C compilers without `<stdbool.h>`.
    pub ok: u8,
    /// The stable error code (e.g. `"PARSE"`, `"KEY"`, `"INTERNAL"`) or `""` on success. Host-owned.
    pub error_code: *mut c_char,
    /// The human-readable error message or `""` on success. Host-owned.
    pub error_message: *mut c_char,
    /// The JSON output for ops that return data, else NULL. Host-owned.
    pub data_json: *mut c_char,
    /// The opaque handle for ops that return one, else NULL. Freed with
    /// [`ootle_partial_transaction_free`], **not** by [`ootle_result_free`].
    pub handle: *mut OotlePartialTransaction,
}

// --- Envelope constructors ----------------------------------------------------------------------

/// Allocates a heap C string. An interior NUL (which `CString::new` rejects) collapses to `""` rather
/// than failing — these strings are facade-controlled JSON/codes/messages and never legitimately hold
/// a NUL, so this is purely defensive.
fn to_c_string(s: &str) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => CString::new("").expect("empty string is always valid").into_raw(),
    }
}

impl OotleResult {
    /// A success envelope carrying a JSON payload (`data_json`), no handle.
    pub(crate) fn ok_json(json: &str) -> Self {
        OotleResult {
            ok: 1,
            error_code: to_c_string(""),
            error_message: to_c_string(""),
            data_json: to_c_string(json),
            handle: ptr::null_mut(),
        }
    }

    /// A success envelope carrying a JSON payload **and** an opaque handle (the two-phase ops).
    fn ok_json_handle(json: &str, handle: PartialTransaction) -> Self {
        OotleResult {
            ok: 1,
            error_code: to_c_string(""),
            error_message: to_c_string(""),
            data_json: to_c_string(json),
            handle: Box::into_raw(Box::new(OotlePartialTransaction {
                kind: HandleKind::Public,
                inner: handle,
            })),
        }
    }

    /// A success envelope carrying the opaque stealth handle (in `handle`) **and** a JSON payload
    /// (`data_json`) — used by the stealth two-phase build (the want list) and apply (the resolution
    /// status). The handle wraps a
    /// [`StealthHandleState`](crate::stealth_abi::StealthHandleState) (resolver or ready).
    ///
    /// The envelope's `handle` field is typed as `*mut OotlePartialTransaction`, but the stealth flow
    /// uses a **distinct** opaque handle
    /// ([`OotleStealthPartialTransaction`](crate::stealth_abi::OotleStealthPartialTransaction)). Cross-casting
    /// through the shared field keeps the envelope layout identical for cgo while preserving the one-handle-field
    /// contract. The host should pass the returned pointer to the `ootle_*_stealth*` consumers /
    /// `ootle_stealth_partial_transaction_free` (each casts it back to the stealth type).
    ///
    /// Misrouting is a **deterministic error**: the boxed struct carries
    /// [`HandleKind::Stealth`] as its first field, and every consumer/free reads the kind through
    /// [`HandleHeader`] before taking ownership — routing this handle to a public-path consumer/free
    /// returns an `INVALID` envelope and leaves the handle intact (no bad deref, no bad free). See the
    /// stealth module docs and [`HandleKind`].
    pub(crate) fn ok_stealth_handle_json(json: &str, state: crate::stealth_abi::StealthHandleState) -> Self {
        let stealth = Box::into_raw(Box::new(crate::stealth_abi::OotleStealthPartialTransaction {
            kind: HandleKind::Stealth,
            inner: state,
        }));
        OotleResult {
            ok: 1,
            error_code: to_c_string(""),
            error_message: to_c_string(""),
            data_json: to_c_string(json),
            handle: stealth as *mut OotlePartialTransaction,
        }
    }

    /// An error envelope with a stable code + message, no payload or handle.
    pub(crate) fn err(code: &str, message: &str) -> Self {
        OotleResult {
            ok: 0,
            error_code: to_c_string(code),
            error_message: to_c_string(message),
            data_json: ptr::null_mut(),
            handle: ptr::null_mut(),
        }
    }

    /// Maps a core error to an error envelope (its stable code + display message).
    pub(crate) fn from_core_err(e: &OotleSdkError) -> Self {
        Self::err(e.code(), &e.to_string())
    }
}

// --- Input helpers --------------------------------------------------------------------------------

/// Reads a required `*const c_char` into a `&str`, mapping NULL / invalid UTF-8 to an `INVALID`
/// envelope. `Ok(&str)` on success, `Err(OotleResult)` (already an error envelope) otherwise.
///
/// # Safety
/// `ptr`, if non-null, must point to a valid NUL-terminated C string that outlives the borrow.
pub(crate) unsafe fn required_str<'a>(ptr: *const c_char, arg: &str) -> Result<&'a str, OotleResult> {
    if ptr.is_null() {
        return Err(OotleResult::err(
            "INVALID",
            &format!("argument `{arg}` must not be null"),
        ));
    }
    // SAFETY: caller guarantees a valid NUL-terminated string for the borrow's lifetime.
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => Ok(s),
        Err(e) => Err(OotleResult::err(
            "INVALID",
            &format!("argument `{arg}` is not valid UTF-8: {e}"),
        )),
    }
}

/// Reads an **optional** `*const c_char`: a NULL pointer is `Ok(None)` (the absent case), a non-null
/// pointer is read as a `&str` (mapping invalid UTF-8 to an `INVALID` envelope). Used for nullable
/// FFI string args (e.g. an optional `pay_ref_hex`).
///
/// # Safety
/// `ptr`, if non-null, must point to a valid NUL-terminated C string that outlives the borrow.
pub(crate) unsafe fn optional_str<'a>(ptr: *const c_char, arg: &str) -> Result<Option<&'a str>, OotleResult> {
    if ptr.is_null() {
        return Ok(None);
    }
    // SAFETY: caller guarantees a valid NUL-terminated string for the borrow's lifetime.
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => Ok(Some(s)),
        Err(e) => Err(OotleResult::err(
            "INVALID",
            &format!("argument `{arg}` is not valid UTF-8: {e}"),
        )),
    }
}

/// Deserializes a JSON argument, mapping a failure to a `PARSE` envelope.
pub(crate) fn parse_json<'de, T: Deserialize<'de>>(json: &'de str, what: &str) -> Result<T, OotleResult> {
    serde_json::from_str(json).map_err(|e| OotleResult::err("PARSE", &format!("invalid {what} JSON: {e}")))
}

/// Serializes a core output to JSON, mapping a (practically impossible) failure to an `ENCODING`
/// envelope.
pub(crate) fn output_json<T: serde::Serialize>(value: &T, what: &str) -> Result<String, OotleResult> {
    serde_json::to_string(value).map_err(|e| OotleResult::err("ENCODING", &format!("failed to serialize {what}: {e}")))
}

// --- Facade-local key mirrors (the core key bundles do not derive Deserialize) --------------------

/// Wire mirror of [`PublicTransferKeys`]: just the account secret, lowercase hex.
#[derive(Debug, Deserialize)]
struct ProductionKeysJson {
    account_secret: SecretKeyBytes,
}

impl ProductionKeysJson {
    fn into_core(self) -> PublicTransferKeys {
        PublicTransferKeys::new(self.account_secret)
    }
}

/// Wire form of a minted account keypair: `{"account_secret":"<hex32>","account_public_key":"<hex32>"}`
/// (lowercase hex). The core [`OotleKeypair`] has no facing serde shape (its secret half does not
/// derive `Serialize`), so this named-field mirror carries it.
#[derive(Debug, serde::Serialize)]
struct AccountKeyPairJson {
    account_secret: SecretKeyBytes,
    account_public_key: PublicKeyBytes,
}

impl AccountKeyPairJson {
    fn from_core(kp: OotleKeypair) -> Self {
        Self {
            account_secret: kp.secret,
            account_public_key: kp.public_key,
        }
    }
}

/// Wire form of a minted view keypair: `{"view_secret":"<hex32>","view_public_key":"<hex32>"}`
/// (lowercase hex).
#[derive(Debug, serde::Serialize)]
struct ViewKeyPairJson {
    view_secret: SecretKeyBytes,
    view_public_key: PublicKeyBytes,
}

impl ViewKeyPairJson {
    fn from_core(kp: OotleKeypair) -> Self {
        Self {
            view_secret: kp.secret,
            view_public_key: kp.public_key,
        }
    }
}

/// Decodes a `seed_hex` argument into a fixed 32-byte seed, mapping any bad/odd/uppercase hex or a
/// wrong length to a `PARSE` envelope.
pub(crate) fn seed_from_hex(seed_hex: &str) -> Result<[u8; 32], OotleResult> {
    if seed_hex.chars().any(|c| c.is_ascii_uppercase()) {
        return Err(OotleResult::err("PARSE", "seed_hex must be lowercase hex"));
    }
    let bytes = hex::decode(seed_hex).map_err(|e| OotleResult::err("PARSE", &format!("invalid seed hex: {e}")))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| OotleResult::err("PARSE", &format!("seed must be 32 bytes, got {}", v.len())))
}

/// Decodes a public-key hex argument into a 32-byte [`PublicKeyBytes`], mapping any bad/odd/uppercase
/// hex or a wrong length to a `PARSE` envelope. The bytes are **not** validated as a canonical curve
/// point: the engine derivation hashes them as-is, so there is no `KEY` path here.
fn public_key_from_hex(pk_hex: &str, arg: &str) -> Result<PublicKeyBytes, OotleResult> {
    if pk_hex.chars().any(|c| c.is_ascii_uppercase()) {
        return Err(OotleResult::err("PARSE", &format!("{arg} must be lowercase hex")));
    }
    let bytes = hex::decode(pk_hex).map_err(|e| OotleResult::err("PARSE", &format!("invalid {arg} hex: {e}")))?;
    if bytes.len() != PublicKeyBytes::LEN {
        return Err(OotleResult::err(
            "PARSE",
            &format!("{arg} must be {} bytes, got {}", PublicKeyBytes::LEN, bytes.len()),
        ));
    }
    // Width is checked above, so `from_bytes` cannot fail here.
    PublicKeyBytes::from_bytes(&bytes).map_err(|e| OotleResult::err("PARSE", &format!("invalid {arg}: {e}")))
}

/// Decodes a secret-key hex argument into a 32-byte [`SecretKeyBytes`], mapping any bad/odd/uppercase
/// hex or a wrong length to a `KEY` envelope (bad key material maps to `KEY`, not `PARSE`; the core's
/// canonical-scalar check at sign time also surfaces as `KEY`). The returned newtype is
/// `ZeroizeOnDrop`; the facade does not zero the incoming C string.
fn secret_key_from_hex(sk_hex: &str, arg: &str) -> Result<SecretKeyBytes, OotleResult> {
    if sk_hex.chars().any(|c| c.is_ascii_uppercase()) {
        return Err(OotleResult::err("KEY", &format!("{arg} must be lowercase hex")));
    }
    let bytes = hex::decode(sk_hex).map_err(|e| OotleResult::err("KEY", &format!("invalid {arg} hex: {e}")))?;
    SecretKeyBytes::from_bytes(&bytes).map_err(|e| OotleResult::err("KEY", &format!("invalid {arg}: {e}")))
}

/// Converts a network discriminant byte into the boundary [`Network`], mapping an unknown byte to an
/// `INVALID` envelope.
pub(crate) fn network_from_byte(byte: u8) -> Result<Network, OotleResult> {
    for n in [
        Network::MainNet,
        Network::StageNet,
        Network::NextNet,
        Network::LocalNet,
        Network::Igor,
        Network::Esmeralda,
    ] {
        if n.as_byte() == byte {
            return Ok(n);
        }
    }
    Err(OotleResult::err(
        "INVALID",
        &format!("unknown network discriminant byte: {byte}"),
    ))
}

/// Runs `f` inside a panic guard, converting any panic into an `INTERNAL` envelope so a panic never
/// unwinds across the C boundary.
///
/// `AssertUnwindSafe` is sound here because each entry point's closure owns its captured state
/// **exclusively** for the duration of the call — in particular a consumed `PartialTransaction` is
/// moved into the closure (via `Box::from_raw`) and never shared, so a panic mid-call cannot leave
/// any caller-observable value in a torn state. If a future change makes the closure capture shared
/// or borrowed-across-the-boundary state, re-audit this assertion rather than widening it.
pub(crate) fn guarded(f: impl FnOnce() -> OotleResult) -> OotleResult {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => OotleResult::err("INTERNAL", "internal error: a panic was caught at the FFI boundary"),
    }
}

/// Collapses a `Result<OotleResult, OotleResult>` (the early-return error-envelope pattern) into the
/// single envelope value the entry points return.
pub(crate) fn flatten(r: Result<OotleResult, OotleResult>) -> OotleResult {
    r.unwrap_or_else(|e| e)
}

// --- One-shot ops ---------------------------------------------------------------------------------

/// Builds, seal+encodes a public transfer in one call (random-nonce default: a fresh OS-RNG seed).
///
/// `intent_json` is a `PublicTransferIntent`; `keys_json` is `{account_secret}` (lowercase hex). On
/// success `data_json` is the `EncodedPublicTransfer` (`{encoded_transaction, transaction_id}`,
/// lowercase hex). The bytes/id are **not** reproducible (random-nonce seal) — the safe default for
/// real submission.
///
/// # Safety
/// `intent_json` and `keys_json` must be valid NUL-terminated UTF-8 C strings. The returned envelope
/// must be freed with [`ootle_result_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_build_and_encode_public_transfer(
    network: u8,
    intent_json: *const c_char,
    keys_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let intent_json = unsafe { required_str(intent_json, "intent_json") }?;
            let keys_json = unsafe { required_str(keys_json, "keys_json") }?;
            let intent: PublicTransferIntent = parse_json(intent_json, "intent")?;
            let keys: ProductionKeysJson = parse_json(keys_json, "keys")?;
            match build_and_encode_public_transfer(network, &intent, &keys.into_core()) {
                Ok(encoded) => Ok(OotleResult::ok_json(&output_json(&encoded, "encoded transfer")?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Identity keygen (group A) --------------------------------------------------------------------

/// Mints a fresh **account** keypair from `OsRng` (production). On success `data_json` is
/// `{"account_secret":"<hex32>","account_public_key":"<hex32>"}` (lowercase hex); no handle. The
/// bytes are non-reproducible by design.
///
/// # Secrets
/// `data_json` carries the **plaintext** account secret. The facade does not zeroize the returned C
/// string (it is owned by the host); the host is responsible for scrubbing / limiting the lifetime of
/// the buffer after reading the secret — same posture as the stealth fns for incoming secret strings.
///
/// # Safety
/// The returned envelope must be freed with [`ootle_result_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_generate_account_key() -> OotleResult {
    guarded(|| {
        flatten((|| {
            let kp = AccountKeyPairJson::from_core(generate_account_keypair());
            Ok(OotleResult::ok_json(&output_json(&kp, "account keypair")?))
        })())
    })
}

/// Mints a fresh **view** keypair from `OsRng` (production). On success `data_json` is
/// `{"view_secret":"<hex32>","view_public_key":"<hex32>"}` (lowercase hex); no handle.
/// Non-reproducible by design.
///
/// # Secrets
/// `data_json` carries the **plaintext** view secret; the host must scrub it (see
/// [`ootle_generate_account_key`]).
///
/// # Safety
/// As [`ootle_generate_account_key`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_generate_view_key() -> OotleResult {
    guarded(|| {
        flatten((|| {
            let kp = ViewKeyPairJson::from_core(generate_view_keypair());
            Ok(OotleResult::ok_json(&output_json(&kp, "view keypair")?))
        })())
    })
}

/// Deterministically derives the **account** keypair from a 32-byte seed (no RNG). `seed_hex` is the
/// lowercase-hex 32-byte seed. On success `data_json` is
/// `{"account_secret":"<hex32>","account_public_key":"<hex32>"}`. Reproducible byte-for-byte.
///
/// Bad / odd / uppercase / wrong-length `seed_hex` ⇒ `"PARSE"`.
///
/// # Secrets
/// `data_json` carries the **plaintext** account secret; the host must scrub it (see
/// [`ootle_generate_account_key`]). The facade does not zeroize the incoming `seed_hex` C string either.
///
/// # Safety
/// `seed_hex` must be a valid NUL-terminated UTF-8 C string. The returned envelope must be freed with
/// [`ootle_result_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_derive_account_key_from_seed(seed_hex: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let seed_hex = unsafe { required_str(seed_hex, "seed_hex") }?;
            let seed = seed_from_hex(seed_hex)?;
            match derive_account_keypair_from_seed(&seed) {
                Ok(kp) => Ok(OotleResult::ok_json(&output_json(
                    &AccountKeyPairJson::from_core(kp),
                    "account keypair",
                )?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Deterministically derives the **view** keypair from a 32-byte seed (no RNG). `seed_hex` is the
/// lowercase-hex 32-byte seed. On success `data_json` is
/// `{"view_secret":"<hex32>","view_public_key":"<hex32>"}`. Reproducible byte-for-byte.
///
/// Bad / odd / uppercase / wrong-length `seed_hex` ⇒ `"PARSE"`.
///
/// # Secrets
/// `data_json` carries the **plaintext** view secret; the host must scrub it (see
/// [`ootle_generate_account_key`]).
///
/// # Safety
/// As [`ootle_derive_account_key_from_seed`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_derive_view_key_from_seed(seed_hex: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let seed_hex = unsafe { required_str(seed_hex, "seed_hex") }?;
            let seed = seed_from_hex(seed_hex)?;
            match derive_view_keypair_from_seed(&seed) {
                Ok(kp) => Ok(OotleResult::ok_json(&output_json(
                    &ViewKeyPairJson::from_core(kp),
                    "view keypair",
                )?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Address derivation (group A) -----------------------------------------------------------------

/// Derives the canonical **account component address** from an account public key.
///
/// `account_public_key_hex` is the lowercase-hex 32-byte account public key. On success `data_json` is
/// `{"component_address":"component_<hex>"}`; no handle. Uses the same engine derivation as the
/// transfer builder, so a derived address matches where the builder places a recipient account.
/// Network-independent (template + pk only).
///
/// Bad / odd / uppercase / wrong-length `account_public_key_hex` ⇒ `"PARSE"`. The bytes are not
/// validated as a canonical curve point (the engine hashes them as-is), so there is no `"KEY"` path.
///
/// # Safety
/// `account_public_key_hex` must be a valid NUL-terminated UTF-8 C string. The returned envelope must
/// be freed with [`ootle_result_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_derive_account_address(account_public_key_hex: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let pk_hex = unsafe { required_str(account_public_key_hex, "account_public_key_hex") }?;
            let pk = public_key_from_hex(pk_hex, "account_public_key_hex")?;
            match derive_account_address(&pk) {
                Ok(component) => {
                    let body = serde_json::json!({ "component_address": component.as_str() });
                    Ok(OotleResult::ok_json(&output_json(&body, "component address")?))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Address codec (group A) ----------------------------------------------------------------------

/// Parses an address string into a kind-tagged record, dispatching on prefix.
///
/// `address_str` is either a `component_<hex>` / `resource_<hex>` engine substate id **or** an
/// `otl_…` bech32m identity / payment address. On success `data_json` is the kind-tagged
/// [`ParsedAddress`](ootle_sdk_core::ParsedAddress):
/// - substate: `{"kind":"component"|"resource","canonical":"…_<hex>"}`;
/// - identity: `{"kind":"identity","network":"<keyword>","account_key":"<hex>","view_only_key":"<hex>",
///   "pay_ref":"<hex>"?,"bech32m":"otl_…"}` (`pay_ref` is absent when none).
///
/// An unknown prefix, a malformed substate id, or a bad bech32m string (checksum / HRP / length /
/// oversize pay_ref) ⇒ `"PARSE"`; no handle.
///
/// # Safety
/// `address_str` must be a valid NUL-terminated UTF-8 C string. The returned envelope must be freed
/// with [`ootle_result_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_parse_address(address_str: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let address = unsafe { required_str(address_str, "address_str") }?;
            match parse_address(address) {
                Ok(parsed) => Ok(OotleResult::ok_json(&output_json(&parsed, "parsed address")?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Formats an `otl_…` identity / payment address (bech32m) from its parts.
///
/// `network` is the L1 discriminant byte (selects the network-qualified HRP — `otl_` MainNet,
/// `otl_loc_` LocalNet, `otl_esm_` Esmeralda, …). `account_key_hex` / `view_only_key_hex` are the
/// lowercase-hex 32-byte public keys. `pay_ref_hex` is a **nullable** lowercase-hex payment reference
/// (pass NULL for none); over 64 bytes ⇒ `"PARSE"`. On success `data_json` is `{"bech32m":"otl_…"}`;
/// no handle.
///
/// Bad / odd / uppercase key hex or pay_ref hex ⇒ `"PARSE"`.
///
/// # Safety
/// `account_key_hex` and `view_only_key_hex` must be valid NUL-terminated UTF-8 C strings.
/// `pay_ref_hex` must be NULL or such a string. The returned envelope must be freed with
/// [`ootle_result_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_format_identity_address(
    network: u8,
    account_key_hex: *const c_char,
    view_only_key_hex: *const c_char,
    pay_ref_hex: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let account_hex = unsafe { required_str(account_key_hex, "account_key_hex") }?;
            let view_hex = unsafe { required_str(view_only_key_hex, "view_only_key_hex") }?;
            // The pay_ref is optional: NULL ⇒ None (handle the null before any required_str path).
            let pay_ref = unsafe { optional_str(pay_ref_hex, "pay_ref_hex") }?;

            let account = public_key_from_hex(account_hex, "account_key_hex")?;
            let view = public_key_from_hex(view_hex, "view_only_key_hex")?;

            match format_identity_address(network, &account, &view, pay_ref) {
                Ok(bech32m) => {
                    let body = serde_json::json!({ "bech32m": bech32m });
                    Ok(OotleResult::ok_json(&output_json(&body, "identity address")?))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Two-phase ops --------------------------------------------------------------------------------

/// Builds the public-transfer unsigned tx + the want list to resolve it.
///
/// On success the envelope carries the opaque handle (in `handle`) and the want list in `data_json`
/// (`{"want_list":[…]}` — the serde form of `WantList`'s inner `Vec<WantItem>`). The host fetches the
/// wanted substates, then drives [`ootle_apply_fetched_substates`].
///
/// # Safety
/// `intent_json` must be a valid NUL-terminated UTF-8 C string. The returned envelope must be freed
/// with [`ootle_result_free`]; the returned `handle` must be consumed by a later call or freed with
/// [`ootle_partial_transaction_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_build_unsigned(network: u8, intent_json: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let intent_json = unsafe { required_str(intent_json, "intent_json") }?;
            let intent: PublicTransferIntent = parse_json(intent_json, "intent")?;
            match build_public_transfer_unsigned_with_wants(network, &intent) {
                Ok((partial, want_list)) => {
                    let body = serde_json::json!({ "want_list": want_list.0 });
                    Ok(OotleResult::ok_json_handle(&output_json(&body, "want list")?, partial))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Builds an unsigned tx from a **generic instruction list** + the want list to resolve it.
///
/// Lowers a generic instruction list + typed-arg DSL ([`GenericTransactionIntent`]) into an unsigned
/// transaction. Returns the same opaque handle + want-list envelope as [`ootle_build_unsigned`]; finish
/// it with [`ootle_apply_fetched_substates`] then [`ootle_seal_and_encode`].
///
/// On success the envelope carries the opaque handle (in `handle`) and the want list in `data_json`
/// (`{"want_list":[…]}`). Bad intent JSON ⇒ `"PARSE"`; an out-of-range blob index / unbound workspace
/// label ⇒ `"VALIDATION"` (the core's structural checks).
///
/// # Safety
/// `intent_json` must be a valid NUL-terminated UTF-8 C string. The returned envelope must be freed
/// with [`ootle_result_free`]; the returned `handle` must be consumed by a later call or freed with
/// [`ootle_partial_transaction_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_build_unsigned_instructions(network: u8, intent_json: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let intent_json = unsafe { required_str(intent_json, "intent_json") }?;
            let intent: GenericTransactionIntent = parse_json(intent_json, "generic intent")?;
            match build_unsigned_instructions_with_wants(network, &intent) {
                Ok((partial, want_list)) => {
                    let body = serde_json::json!({ "want_list": want_list.0 });
                    Ok(OotleResult::ok_json_handle(&output_json(&body, "want list")?, partial))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Builds an unsigned **faucet claim** + the want list to resolve it.
///
/// Emits a complete self-funding claim (create the recipient account, fund it from the network faucet,
/// pay the fee from it) with the faucet's full input set derived internally. Returns the same opaque
/// handle + want-list envelope as [`ootle_build_unsigned`]; finish it with
/// [`ootle_apply_fetched_substates`] + [`ootle_seal_and_encode`].
///
/// `intent_json` is a [`FaucetClaimIntent`] (`{"recipient_public_key":"<hex>","fee":<µTari>,…}`). On
/// success the envelope carries the handle and `{"want_list":[…]}`. Bad intent JSON ⇒ `"PARSE"`.
///
/// # Safety
/// `intent_json` must be a valid NUL-terminated UTF-8 C string. The returned envelope must be freed
/// with [`ootle_result_free`]; the returned `handle` must be consumed by a later call or freed with
/// [`ootle_partial_transaction_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_build_faucet_claim(network: u8, intent_json: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let network = network_from_byte(network)?;
            let intent_json = unsafe { required_str(intent_json, "intent_json") }?;
            let intent: FaucetClaimIntent = parse_json(intent_json, "faucet claim intent")?;
            match build_faucet_claim_with_wants(network, &intent) {
                Ok((partial, want_list)) => {
                    let body = serde_json::json!({ "want_list": want_list.0 });
                    Ok(OotleResult::ok_json_handle(&output_json(&body, "want list")?, partial))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Applies a fetched batch of substates to the partial and reports resolution status.
///
/// **Consumes** `handle` (it is taken by value — treat the pointer you passed as invalid afterwards,
/// even on error). `fetched_json` is a JSON array of `FetchedSubstate`. On success the envelope
/// carries a (possibly new) handle to thread forward and `data_json`:
/// - `{"status":"resolved"}` — ready to seal via [`ootle_seal_and_encode`]; or
/// - `{"status":"need_more","want_list":[…],"fetch_ids":[…]}` — fetch the substates named in **`fetch_ids`** (the
///   authoritative concrete next-fetch set, including vault ids the core discovered inside a fetched component) and
///   call this again on the returned handle. `want_list` is the informational semantic remainder; fetch `fetch_ids`,
///   not the want-list seeds.
///
/// On error the input handle is still consumed and freed; the returned envelope carries no handle.
///
/// # Safety
/// `handle` must be a non-null pointer previously returned by [`ootle_build_unsigned`] /
/// [`ootle_apply_fetched_substates`] and not yet consumed. `fetched_json` must be a valid
/// NUL-terminated UTF-8 C string. The returned envelope must be freed with [`ootle_result_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_apply_fetched_substates(
    handle: *mut OotlePartialTransaction,
    fetched_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        if handle.is_null() {
            return OotleResult::err("INVALID", "argument `handle` must not be null");
        }
        // Validate the kind tag through the shared header BEFORE taking ownership: a misrouted
        // (stealth) handle is rejected here and never reconstructed as a `Box<OotlePartialTransaction>`.
        if let Err(e) = unsafe { require_kind(handle, HandleKind::Public) } {
            return e;
        }
        // Take ownership of the handle up front: it is consumed on every path (success or error),
        // matching `apply_fetched_substates`'s by-value signature. The host must not free it again.
        let partial = unsafe { Box::from_raw(handle) }.inner;

        flatten((|| {
            let fetched_json = unsafe { required_str(fetched_json, "fetched_json") }?;
            let fetched: Vec<FetchedSubstate> = parse_json(fetched_json, "fetched substates")?;
            match apply_fetched_substates(partial, &fetched) {
                Ok(Resolution::Resolved(resolved)) => {
                    let body = serde_json::json!({ "status": "resolved" });
                    Ok(OotleResult::ok_json_handle(
                        &output_json(&body, "resolution")?,
                        resolved,
                    ))
                },
                Ok(Resolution::NeedMore {
                    partial,
                    want_list,
                    fetch_ids,
                }) => {
                    // `fetch_ids` is the authoritative next-fetch set; `want_list` is informational.
                    let body = serde_json::json!({
                        "status": "need_more",
                        "want_list": want_list.0,
                        "fetch_ids": fetch_ids,
                    });
                    Ok(OotleResult::ok_json_handle(&output_json(&body, "resolution")?, partial))
                },
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Seals (random-nonce default: a fresh OS-RNG seed) and BOR-encodes a **resolved** partial.
///
/// **Consumes** `handle`. `keys_json` is `{account_secret}` (lowercase hex). On success `data_json` is
/// the `EncodedPublicTransfer`. A still-unresolved partial ⇒ a `RESOLUTION` error (the handle is still
/// consumed). No handle is returned. The bytes/id are not reproducible (random-nonce seal).
///
/// # Safety
/// `handle` must be a non-null, not-yet-consumed pointer from the two-phase ops. `keys_json` must be a
/// valid NUL-terminated UTF-8 C string. The returned envelope must be freed with [`ootle_result_free`].
/// Do **not** free `handle` afterwards — it is consumed here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_seal_and_encode(
    handle: *mut OotlePartialTransaction,
    keys_json: *const c_char,
) -> OotleResult {
    guarded(|| {
        if handle.is_null() {
            return OotleResult::err("INVALID", "argument `handle` must not be null");
        }
        if let Err(e) = unsafe { require_kind(handle, HandleKind::Public) } {
            return e;
        }
        let partial = unsafe { Box::from_raw(handle) }.inner;

        flatten((|| {
            let keys_json = unsafe { required_str(keys_json, "keys_json") }?;
            let keys: ProductionKeysJson = parse_json(keys_json, "keys")?;
            match seal_and_encode_public_transfer(partial, &keys.into_core()) {
                Ok(encoded) => Ok(OotleResult::ok_json(&output_json(&encoded, "encoded transfer")?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- Co-signing (authorize → attach → seal) -------------------------------------------------------

/// Derives the serializable [`UnsignedTransactionRecord`] party A ships to party B, from a resolved
/// public handle. **Borrows** the handle (does **not** consume it) so A can ship the record JSON *and*
/// keep the handle to seal later with [`ootle_seal_and_encode_with_auth`].
///
/// `main_signer_hex` (lowercase hex, nullable) names the co-signer the transaction acts as wherever it
/// needs a single signer (`OwnerRule::OwnedBySigner`, the default owner of new components and
/// resources). NULL declares no main signer. Pass the same value to [`ootle_seal_and_encode_with_auth`].
///
/// On success `data_json` is the `UnsignedTransactionRecord` JSON (`{"unsigned": <UnsignedTransaction>}`).
/// An unresolved partial ⇒ a `RESOLUTION` error (the handle is left intact). Bad `main_signer_hex` ⇒
/// `PARSE`.
///
/// # Safety
/// `handle` must be a non-null, not-yet-consumed public handle. `main_signer_hex` must be NULL or a
/// valid NUL-terminated UTF-8 C string. The returned envelope must be freed with [`ootle_result_free`].
/// The handle is **not** consumed; free it with [`ootle_partial_transaction_free`] (or consume it via a
/// seal fn) exactly once afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_unsigned_record_for_cosign(
    handle: *const OotlePartialTransaction,
    main_signer_hex: *const c_char,
) -> OotleResult {
    guarded(|| {
        if handle.is_null() {
            return OotleResult::err("INVALID", "argument `handle` must not be null");
        }
        if let Err(e) = unsafe { require_kind(handle, HandleKind::Public) } {
            return e;
        }
        // SAFETY: non-null, kind-checked public handle; borrowed (not boxed/consumed).
        let partial: &PartialTransaction = unsafe { &(*handle).inner };
        flatten((|| {
            let main_signer = unsafe { optional_main_signer(main_signer_hex) }?;
            match ootle_sdk_core::unsigned_record_for_cosign(partial, main_signer.as_ref()) {
                Ok(record) => Ok(OotleResult::ok_json(&output_json(&record, "unsigned record")?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Authorizes party A's serialized unsigned transaction record (production path: fresh random Schnorr
/// nonce), committing to A's seal public key. Called by party B.
///
/// `unsigned_json` is the `UnsignedTransactionRecord` JSON A shipped (`{"unsigned": <UnsignedTransaction
/// JSON>}`, from [`ootle_unsigned_record_for_cosign`]). `seal_public_key_hex` is A's seal public key
/// (lowercase hex). `signer_secret_hex` is B's secret key (lowercase hex). On success `data_json` is
/// `{ "authorization": { "public_key": "<hex>", "signature": "<hex>" } }`.
///
/// `network` is accepted for API symmetry (validated against the known discriminants); the signing
/// message is derived from the unsigned record, which already carries the network.
///
/// Bad key hex ⇒ `KEY`; a non-canonical secret scalar ⇒ `KEY`; bad/odd/uppercase seal-pk hex ⇒ `PARSE`;
/// malformed `unsigned_json` ⇒ `PARSE`.
///
/// # Safety
/// `unsigned_json`, `seal_public_key_hex`, and `signer_secret_hex` must each be a valid NUL-terminated
/// UTF-8 C string. The returned envelope must be freed with [`ootle_result_free`]. `signer_secret_hex`
/// crosses as transient hex and is dropped at the end of the call; the facade does not zero it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_add_signature(
    network: u8,
    unsigned_json: *const c_char,
    seal_public_key_hex: *const c_char,
    signer_secret_hex: *const c_char,
) -> OotleResult {
    guarded(|| {
        flatten((|| {
            // Validate the network byte (not used in the crypto, but rejected if unknown).
            let _network = network_from_byte(network)?;
            let unsigned_json = unsafe { required_str(unsigned_json, "unsigned_json") }?;
            let record: UnsignedTransactionRecord = parse_json(unsigned_json, "unsigned record")?;
            let seal_pk_hex = unsafe { required_str(seal_public_key_hex, "seal_public_key_hex") }?;
            let seal_pk = public_key_from_hex(seal_pk_hex, "seal_public_key_hex")?;
            let signer_secret_hex = unsafe { required_str(signer_secret_hex, "signer_secret_hex") }?;
            let signer_secret = secret_key_from_hex(signer_secret_hex, "signer_secret_hex")?;

            match add_signature(&record, &seal_pk, &signer_secret) {
                Ok(auth) => Ok(OotleResult::ok_json(&output_json(
                    &serde_json::json!({ "authorization": auth }),
                    "authorization",
                )?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Seals a resolved partial **with attached authorizations** (random-nonce default: a fresh OS-RNG
/// seed) and BOR-encodes it. **Consumes** `handle` (exactly like [`ootle_seal_and_encode`]).
///
/// `keys_json` is `{account_secret}` (lowercase hex). `authorizations_json` is a JSON array
/// `[{ "public_key": "<hex>", "signature": "<hex>" }, ...]`. An empty array behaves like the plain
/// single-key seal. `main_signer_hex` (nullable) must be the value passed to
/// [`ootle_unsigned_record_for_cosign`]; a main signer that is not one of the authorizations ⇒
/// `INVALID`, and an authorization that does not verify against the sealed transaction (for example
/// one signed over a record with a different main signer) ⇒ `VALIDATION`. On success `data_json` is the
/// `EncodedPublicTransfer`. The bytes/id are not reproducible (random-nonce seal).
///
/// # Safety
/// `handle` must be a non-null, not-yet-consumed public handle. `keys_json` and `authorizations_json`
/// must be valid NUL-terminated UTF-8 C strings, and `main_signer_hex` NULL or one. The returned
/// envelope must be freed with [`ootle_result_free`]. Do **not** free `handle` afterwards — it is
/// consumed here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_seal_and_encode_with_auth(
    handle: *mut OotlePartialTransaction,
    keys_json: *const c_char,
    authorizations_json: *const c_char,
    main_signer_hex: *const c_char,
) -> OotleResult {
    guarded(|| {
        if handle.is_null() {
            return OotleResult::err("INVALID", "argument `handle` must not be null");
        }
        if let Err(e) = unsafe { require_kind(handle, HandleKind::Public) } {
            return e;
        }
        let partial = unsafe { Box::from_raw(handle) }.inner;

        flatten((|| {
            let keys_json = unsafe { required_str(keys_json, "keys_json") }?;
            let keys: ProductionKeysJson = parse_json(keys_json, "keys")?;
            let authorizations_json = unsafe { required_str(authorizations_json, "authorizations_json") }?;
            let authorizations: Vec<Authorization> = parse_json(authorizations_json, "authorizations")?;
            let main_signer = unsafe { optional_main_signer(main_signer_hex) }?;
            match seal_and_encode_with_auth(partial, &keys.into_core(), &authorizations, main_signer.as_ref()) {
                Ok(encoded) => Ok(OotleResult::ok_json(&output_json(&encoded, "encoded transfer")?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

/// Reads the nullable `main_signer_hex` co-sign argument.
///
/// # Safety
/// `ptr` must be NULL or a valid NUL-terminated C string.
unsafe fn optional_main_signer(ptr: *const c_char) -> Result<Option<PublicKeyBytes>, OotleResult> {
    unsafe { optional_str(ptr, "main_signer_hex") }?
        .map(|hex| public_key_from_hex(hex, "main_signer_hex"))
        .transpose()
}

// --- Result parsing -------------------------------------------------------------------------------

/// Parses a raw indexer finalized-result JSON string into the typed `FinalizedResult`.
///
/// `raw_json` is the indexer's result JSON verbatim. On success `data_json` is the `FinalizedResult`
/// serialized as JSON.
///
/// # Safety
/// `raw_json` must be a valid NUL-terminated UTF-8 C string. The returned envelope must be freed with
/// [`ootle_result_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_parse_finalized_result(raw_json: *const c_char) -> OotleResult {
    guarded(|| {
        flatten((|| {
            let raw_json = unsafe { required_str(raw_json, "raw_json") }?;
            match parse_finalized_result(raw_json) {
                Ok(finalized) => Ok(OotleResult::ok_json(&output_json(&finalized, "finalized result")?)),
                Err(e) => Ok(OotleResult::from_core_err(&e)),
            }
        })())
    })
}

// --- ABI tag + free functions ---------------------------------------------------------------------

/// Returns the stable ABI version tag (a static NUL-terminated C string). **Do not free** the
/// returned pointer — it points at static storage, not a host-owned allocation.
#[unsafe(no_mangle)]
pub extern "C" fn ootle_abi_version() -> *const c_char {
    // A static, NUL-terminated byte string — its pointer is valid for the program's lifetime and is
    // never freed by the host.
    ABI_VERSION.as_ptr() as *const c_char
}

/// Frees a single heap C string previously returned by this library (e.g. an envelope field handed
/// out individually). Null-safe; call **exactly once** per string. Most callers use
/// [`ootle_result_free`] instead, which frees a whole envelope's strings.
///
/// # Safety
/// `s` must be either null or a pointer obtained from this library and not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: pointer originated from `CString::into_raw` in this library.
    drop(unsafe { CString::from_raw(s) });
}

/// Frees the three heap C strings owned by an [`OotleResult`] (`error_code`, `error_message`,
/// `data_json`). Null fields are skipped. Does **not** free `handle` — use
/// [`ootle_partial_transaction_free`] for that. Call **exactly once** per returned envelope.
///
/// # Safety
/// Each non-null string field of `result` must be a pointer obtained from this library and not yet
/// freed. After this call the envelope's string fields are dangling and must not be used.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_result_free(result: OotleResult) {
    unsafe {
        ootle_string_free(result.error_code);
        ootle_string_free(result.error_message);
        ootle_string_free(result.data_json);
    }
    // `handle` is intentionally left untouched — it has its own lifecycle.
}

/// Frees an opaque [`OotlePartialTransaction`] handle. Null-safe; call **exactly once**, and **only**
/// for a handle that was never consumed by [`ootle_apply_fetched_substates`] /
/// [`ootle_seal_and_encode`] (those take the handle by value). Freeing a consumed handle is a
/// use-after-free.
///
/// **Kind-guarded:** if the handle is actually a stealth handle (misrouted from the `ootle_*_stealth*`
/// flow), it is **not** freed — the call is a deterministic no-op that leaves the handle intact for the
/// correct free fn ([`ootle_stealth_partial_transaction_free`]). This prevents a bad free / type
/// confusion.
///
/// # Safety
/// `handle` must be either null or a pointer obtained from this library and not yet consumed or freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ootle_partial_transaction_free(handle: *mut OotlePartialTransaction) {
    if handle.is_null() {
        return;
    }
    // Refuse a wrong-kind (stealth) handle: do NOT `Box::from_raw` it as a public handle (that would
    // be a bad free / type confusion). The handle is left intact for the correct free fn.
    if unsafe { handle_kind(handle) } != HandleKind::Public {
        return;
    }
    // SAFETY: pointer originated from `Box::into_raw` in this library, is the matching kind, and has
    // not been consumed.
    drop(unsafe { Box::from_raw(handle) });
}
