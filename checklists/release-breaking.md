# Breaking / protocol release checklist

Runs **alongside** [release.md](release.md), not instead of it. Work through this one first: most of
its items are decisions that have to be made and merged before the tag exists.

"Breaking" means any of: a consensus or engine change that alters execution, a hashed-schema change,
an on-chain ABI change, a config struct change, or a public API change in a crate or SDK surface.

Shared reference: [README.md](README.md).

---

## 1. Decide the activation, before the tag

- [ ] **Does this need a `ProtocolVersion` activation?** Hashed-schema changes do; ungated engine
      changes (metering, weight, event payloads, auth model) switch at *binary deployment* and
      cannot be epoch-gated. **(human)**
- [ ] Set the epoch **per network** in `crates/engine_types/src/protocol_version.rs`. An epoch
      carried over from the previous release activates in the past or never. Pick it far enough
      ahead that the fleet is upgraded first — past releases used ~1.5 h.
- [ ] Confirm both preconditions hold, or the activation is unsound:
      1. every validator runs the new binary **before** the activation epoch
         (`check_activation_schedule` refuses to start a node whose schedule disagrees with what it
         has already run past);
      2. the network never ran a binary carrying the new hashed fields while still on the old
         version.
- [ ] For an ungated engine change, state plainly in the upgrade notes that **the fleet upgrades and
      restarts together** — two validators on different binaries compute different receipts for the
      same transaction.

## 2. Write real upgrade notes

The `### ⚠️ Upgrade notes` block in `CHANGELOG.md` is the deliverable other teams act on. Cover,
where they apply:

- [ ] What breaks for **operators**: removed config keys (structs are `deny_unknown_fields`, so a
      stale key fails to load), changed limits, caches to wipe, data directories to delete.
- [ ] What breaks for **template authors**: engine ops, ABI sections, auth rules, size caps.
- [ ] What breaks for **app / wallet developers**: fee model and estimates, event payloads and
      filters, reject/abort reasons, account methods (e.g. the `_with_auth` variants).
- [ ] What breaks **without a reset**, and is accepted as a testnet cost.

## 3. SDK surfaces

- [ ] **FFI C ABI** — if the `extern "C"` surface changed, bump `ABI_VERSION` in
      `crates/ootle_sdk_ffi_c/src/c_abi.rs` **and** `ExpectedABIVersion` in ootle-go's
      `internal/cffi/cffi.go`. That tag is the only thing standing between a host and a silently
      mismatched lib.
- [ ] **Golden vectors** — if transaction encoding or result parsing changed, regenerate
      `crates/ootle_sdk_core/fixtures/` and sync them into ootle-go (`scripts/sync_fixtures.sh`) and
      ootle-py (`scripts/regen_fixtures.py`).
- [ ] **A behaviour change with no ABI change still breaks hosts.** Anything a host matches on by
      name or shape — fee source names, event payload fields, reject reasons — is a break the ABI
      tag will not catch, so it belongs in the upgrade notes and in each SDK's changelog.
- [ ] **ootle.ts** — a breaking bindings change means a breaking `@tari-project/ootle` release, not
      a patch. Say so in its changelog.
- [ ] **ootle-py** — the vendored wasm blob carries engine behaviour, so the same break reaches
      Python. Bump it as a breaking release and regenerate its fixtures.
- [ ] **Community templates** — a template ABI break means templates must be rebuilt and
      republished. Flag it. **(human)**
- [ ] **Minotari base layer** — the Minotari wallet writes burn claim proofs with `tari_sidechain`
      from crates.io. A change to `CompleteClaimBurnProof` or the types it carries breaks the claim
      files it writes: say so in the upgrade notes, and plan the base layer release that takes the
      new pin. **(human)**

## 4. crates.io cascade

- [ ] ```sh
      ./scripts/crate_versioning.py impact <crate> --breaking
      ```
      Apply the tier-3 rollup and only the independent bumps it asks for. Crates listed as "already
      covered by an unreleased bump" need nothing — bumping them anyway burns a version.

## 5. Back to the main checklist

- [ ] Continue with [release.md](release.md) from its pre-flight section. Its gate
      (`./scripts/release_check.py`) will re-flag a touched activation schedule; that warning is
      expected here — confirm the epochs are the ones you chose above, not inherited.
