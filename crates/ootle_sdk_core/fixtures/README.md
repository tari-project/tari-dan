# Golden-vector fixtures

Language-neutral `input → expected output` fixtures for `ootle_sdk_core` (Shared Contract E). The
core is the **single source of truth**: a generator drives it to produce `expected`, and a Rust
runner re-runs the core and asserts it reproduces those outputs.

**These same files are consumed by every host SDK** — Go first, then Kotlin, then TS / Python (D4).
So the format contains **no Rust-only constructs**: scalars, hex strings, nested objects only.

## Layout

One JSON file per vector, under `fixtures/<group>/<name>.json` (clean diffs, easy for host SDKs to
pick up). The runner loads **every** `*.json` under `fixtures/` recursively.

| Group | Purpose |
|-------|---------|
| `public_transfer/` | Build + encode a public (account→account) transfer, sealed deterministically. |
| `resolve_public_transfer/` | One-shot resolve (with fetched inputs) + seal + encode of a public transfer. |
| `cosign/` | Multi-party co-sign: add an authorization signature, then seal-with-auth. |
| `stealth_transfer/` | Build + encode a stealth transfer (semantic compare — see below). |
| `stealth_outputs_statement/` | Build the stealth outputs statement + aggregated output mask (semantic compare). |
| `stealth_scan/` | Scan a stealth UTXO for ownership and decode it. |
| `keys/` | Derive account / view keys from a seed (the deterministic KDF keygen). |
| `address_derive/` | Derive an account address from key material. |
| `address_codec/` | Parse and format identity / account addresses. |
| `arg_dsl/` | Encode the manifest-arg DSL values to their canonical CBOR. |
| `substate_decode/` | Decode a substate value to its typed shape. |
| `account_balances/` | Aggregate vault substates into per-resource account balances. |
| `generic_build/` | Build + encode generic instructions and a faucet claim. |
| `parse_finalized_result/` | Parse a finalized `TransactionResult` (structural compare). |

## Format

```jsonc
{
  "name": "public_transfer/single_key_basic", // stable id; shown in failure messages
  "schema_version": 1,                          // integer; bump on format changes
  "provenance": {                               // how/when produced — traceability, not silent magic
    "core_version": "<workspace version>",
    "git_rev": "…",                             // git HEAD at generation (or "unknown")
    "generated_by": "ootle_sdk_core golden-vector generator"
  },
  "compare": "semantic",                        // OPTIONAL — omitted ⇒ "bytes" (the default; see below)
  "operation": "build_and_encode_public_transfer", // the core entry point exercised
  "input": {                                    // *exactly* the arguments to that function
    "network": "esmeralda",
    "intent": { … },                            // the typed PublicTransferIntent (serde)
    "keys": {                                   // pinned, deterministic key + seed material
      "account_secret": "<hex>",
      "seed": "<hex>",                           // 32-byte build seed (see below)
      "seal_secret": "<hex>"                     // OPTIONAL — present only for a separate seal signer
    }
  },
  "expected": {                                 // GENERATOR-OWNED outputs
    "encoded_transaction": "<hex>",             // canonical encoding: lowercase hex
    "transaction_id": "<hex>"                   // 32-byte id, lowercase hex
  }
}
```

The `input` shape varies by operation. The deterministic build/seal vectors carry their entropy as a
single 32-byte **build seed**, never per-nonce material:

- **Public-transfer / resolve / cosign-seal vectors** carry `keys` (`account_secret`, `seed`, optional
  `seal_secret`); the cosign add-signature vector additionally carries the co-signer's
  `cosign_signer_secret` (signing key) and `cosign_signer_seed` (its nonce seed).
- **Stealth-transfer / stealth-seal vectors** carry `stealth_keys` (`account_secret`, `view_secret`, `seed`).
  The view secret keys each spent input's one-time spend key `H(v·R)·G + A`.
- **Stealth-scan vectors** carry `stealth_scan_input` (`network`, `view_secret`, `account_public_key`, `output`,
  `skip_memo`). The scan confirms ownership from the view secret and the account public key alone.
- **Stealth-outputs-statement vectors** carry a bare `stealth_seed`.
- **Key-derivation vectors** (`keys/`) carry a bare `seed` — the host input to the deterministic keygen.

The seed is **not** consumed directly: each seeded entry point expands it in-core through a frozen
domain-separated KDF into all the nonces / blinding factors that build needs. (The contract exists and
is stable across languages; its internals are not re-documented here.)

## Comparison mode (`compare`)

- **`"bytes"` (default — `compare` omitted):** the runner asserts **byte-for-byte** on the raw hex
  strings, never on parsed structures — a parsed compare would hide the exact CBOR/encoding drift this
  harness exists to catch. The parse / decode vectors that produce structured (non-hex) output instead
  compare **canonicalized JSON** structurally.
- **`"semantic"` (the stealth vectors):** the aggregated bulletproof (`agg_range_proof`) and the
  viewable-balance proof (`balance_proof`) are byte-unstable across runs — the range-proof transcript
  draws its own RNG, which the build seed does **not** override. So those vectors are **never**
  byte-compared on the proofs. The runner re-runs the core, cryptographically validates every
  signature on the freshly produced transaction/statement, and structurally compares the remaining
  **deterministic** fields with the byte-unstable ones (proofs + Schnorr signature scalars) nulled out.

### Conventions

- **Bytes are lowercase hex with no `0x` prefix.** Empty buffer = `""`, absent optional = JSON
  `null` (or omitted), so a future Go / Kotlin / Python runner reads the same strings.
- **`input` is fully deterministic.** No "current epoch", no RNG. The pinned **seed** is part of
  `input`: without it neither `encoded_transaction` nor `transaction_id` is reproducible — the
  generator would need RNG and the runner would never match.

### `expected` is generator-owned — never hand-edit it

Humans author `input` (in the generator's seed code); the **generator is the only writer of
`expected`**. If you change `input`, regenerate — do not hand-patch the bytes.

## Regenerating

```sh
OOTLE_REGEN_FIXTURES=1 cargo test -p ootle_sdk_core --test golden_vectors regen_fixtures
```

The generator seeds any missing vector from its `input` seed code, then re-runs the core over every
fixture's `input` and rewrites `expected` + `provenance`. Regeneration is **idempotent**: with no core
change, it produces byte-identical JSON and leaves the working tree clean (enforced by the
`regen_is_idempotent` test, which pins `git_rev` to a fixed value on both sides of the compare so only
the produced bytes are checked — provenance metadata is allowed to drift with `HEAD`).

To pin `git_rev` (e.g. in CI), set `OOTLE_FIXTURE_GIT_REV=<rev>`; otherwise it is taken from
`git rev-parse HEAD`, falling back to `"unknown"`.

## Running the runner

```sh
cargo test -p ootle_sdk_core --test golden_vectors run_golden_vectors
```

On mismatch it panics with the fixture `name`, the file path, and an expected-vs-actual diff. A stale
committed `expected` is a **red build**, not a silent pass.

## Adding a new vector

1. Author the `input` in the generator (`tests/golden_vectors.rs`): add a seed fn, and have
   `regen_fixtures` write it to a new `fixtures/<group>/<name>.json` if absent (mirror an existing
   seed-if-missing block).
2. Run the generator (above) to fill `expected` + `provenance`.
3. Run the runner — it picks up the new file automatically (it globs `*.json`).
4. Commit the JSON. **Do not hand-edit `expected`.**
</content>
</invoke>
