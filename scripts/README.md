# `scripts/` — release & versioning playbook

This directory is the source of truth for **publishing crates to crates.io** and
**reasoning about version bumps** across the workspace. The two scripts share
state (the crate list lives in `publish_crates.py`) so neither drifts.

If you are an AI agent: prefer running these scripts over hand-editing
`Cargo.toml`s. The output is deterministic and accounts for the workspace
versioning rules below.

---

## TL;DR — the two scripts

```sh
# What's in the publish set, in publish order, with current versions + tiers,
# and whether each version is already on crates.io or still pending release
./scripts/crate_versioning.py list

# I bumped crate X. Who else needs to bump, and how?
./scripts/crate_versioning.py impact <crate>              # non-breaking (patch)
./scripts/crate_versioning.py impact <crate> --breaking   # breaking (minor)

# Both consult crates.io. Add --offline to skip it (see "Pending releases" below)
./scripts/crate_versioning.py impact <crate> --breaking --offline

# Once versions are set, publish to crates.io
./scripts/publish_crates.py                  # what would publish (read-only)
./scripts/publish_crates.py --dry-run        # cargo publish --dry-run each crate
./scripts/publish_crates.py --execute        # actually publish
./scripts/publish_crates.py --from <crate> --execute   # resume after a failure
```

---

## Versioning model (must read once)

Every crate in the publish set is `0.y.z`. Cargo's pre-1.0 SemVer rules:

| Bump kind  | Version change             | Dependent impact                                   |
|------------|----------------------------|----------------------------------------------------|
| **patch**  | `0.y.z → 0.y.(z+1)`        | Auto-picked up by `^0.y` pins. No republish needed. |
| **minor**  | `0.y.z → 0.(y+1).0`        | **Breaks** `^0.y` pins. Every direct dependent updates its pin and republishes. |

Crates fall into one of four tiers, grouped by concern (see
`publish_crates.py::TIER_LABELS`). Tier 3 is the **only** workspace-versioned
cohort; every other tier is independently versioned (each crate carries its own
`version` in its `Cargo.toml`):

| Tier | Label      | Policy |
|------|------------|--------|
| 1    | `stable`   | Shared primitives, rarely change (e.g. `tari_bor`, `ootle-network`, `ootle_byte_type`, `ootle_serde`, `tari_ootle_address`). |
| 2    | `template` | The template-authoring API (`tari_template_*`, `tari_ootle_template_*`). |
| 3    | `core`     | Share `[workspace.package].version` in the root `Cargo.toml`. **Move together.** A breaking bump on any tier-3 crate moves the entire cohort. |
| 4    | `wallet`   | Wallet SDK, clients & storage (`tari_ootle_wallet_*`, `tari_*_client`, `ootle-rs`). Decoupled from the core version. |

**Key consequence:** if a tier-3 crate has a breaking change, the workspace
version bumps and **every** tier-3 crate republishes — even the ones whose own
public API didn't change. Independent (tier 1/2/4) crates only republish if they
actually depend on something that moved.

---

## Pending releases (the second must-read)

The tiers above answer *who a change is breaking for*. That is not the same
question as *who still needs a version edit*, and the two come apart whenever a
release is pending.

A version that was never published to crates.io carries no `^0.y` pins. Nobody
depends on it, so nothing can be broken by changing what goes into it. If a
crate's working-tree version is already an unreleased minor ahead of crates.io,
its bump is **already made** — the pending release ships the change under a
number that announces it, and bumping again just burns a version.

This is easy to get wrong, because a whole cohort can sit on unreleased bumps
for weeks. After `tari_bor` 0.15.1 → 0.16.0, `impact tari_bor --breaking` named
18 crates; 17 of them were already on unreleased minors and the real cascade was
one crate.

So `list` and `impact` both query the crates.io sparse index and classify every
crate:

| State | Meaning | Action |
|---|---|---|
| `pending` | the tree's version is not on crates.io **and** breaks a pin on the latest that is | none — the pending release covers it |
| `released` | the tree's version is already on crates.io | must move; the number is spent |
| *short* | a bump is pending, but too small to break the pin it needs to (e.g. `0.15.1` against a published `0.15.0`) | promote it to a minor |

A crate that has never been published is `pending` — its first release announces
everything.

`--offline` skips the lookup. The output then lists the entire cascade whether
or not each bump has already been made, which is the pre-registry behaviour;
useful with no network, misleading otherwise. A failed lookup for an individual
crate is treated as `released`, so the script asks for a bump it may not need
rather than skipping one it does.

---

## `crate_versioning.py` — bump impact analysis

Subcommands:

### `list`

```sh
./scripts/crate_versioning.py list [--offline]
```

Prints the publish set in topological order with current version + tier +
release state + path. Sourced from `cargo metadata` (versions),
`publish_crates.py` (order/tier) and the crates.io sparse index (release state):

```
tari_bor          0.16.0  [stable  ]  pending (crates.io: 0.15.0)  crates/tari_bor
ootle-network      0.2.0  [stable  ]  released                     crates/ootle_network
```

The `pending` column doubles as a release checklist: it is exactly the set of
crates the next `publish_crates.py --execute` will push.

### `deps <crate>`

```sh
./scripts/crate_versioning.py deps ootle-rs
```

Lists the crate's direct dependencies that are in the publish set. Dev-only
deps are flagged `[dev]` — they don't force downstream bumps.

### `dependents <crate> [--transitive]`

```sh
./scripts/crate_versioning.py dependents tari_engine_types
./scripts/crate_versioning.py dependents tari_engine_types --transitive
```

Lists crates that depend on `<crate>`. With `--transitive`, includes indirect
dependents (the forward closure across normal edges).

### `impact <crate> [--breaking]`

The killer command. Given a (possibly-breaking) change to `<crate>`, prints
the full bump plan.

```sh
# Worked example from the user prompt
./scripts/crate_versioning.py impact tari_engine_types --breaking
```

For a **breaking** bump, output is structured as:

1. **Tier 3 (core) cohort** — either the exact `[workspace.package].version` and
   `[workspace.dependencies]` pin updates (`"0.31" → "0.32"`), or a note that the
   workspace version is already an unreleased breaking bump and no rollup is
   needed. The cohort shares one version, so it moves as soon as any single
   member still owes a bump.
2. **Independent (non-core) minor bumps** — independently-versioned crates whose
   published version is spent and so must move, each with the reason (which dep
   it re-exposes) and its registry state.
3. **Independent (non-core) pin updates** — independently-versioned crates that
   don't minor-bump themselves but still need to update pins and republish at least
   a patch. The output lists which pin(s) to bump and a *patch vs minor*
   recommendation (patch is safe; minor is required only if the crate's own
   public API re-exposes the upstream's changed types).
4. **Already covered** — crates the change breaks for whose bump is already made
   and unreleased. Listed for the audit trail; no action.
5. **Dev-only callouts** — informational; dev-only edges never force a bump.
6. **Suggested workflow** — numbered checklist for the bump → format → publish
   loop, covering only the crates that actually need to move. When nothing does,
   it says `No version changes required.` instead.

Without `--breaking`, the output is a single line: it's a patch bump,
dependents auto-pick-up via `^0.y` and nobody else republishes — or, if the
crate's version is already pending release, that no edit is needed at all.

---

## `publish_crates.py` — actual publishing

This script handles the topological publish to crates.io. The crate list,
ordering, and tier tags are defined here — **this file is the single source of
truth.** When you add or remove a published crate, edit only this list;
`crate_versioning.py` reads from it.

Common invocations:

```sh
# See what would publish without doing anything (safe to run any time)
./scripts/publish_crates.py

# Build-test each crate against crates.io without publishing
./scripts/publish_crates.py --dry-run

# Publish everything pending
./scripts/publish_crates.py --execute

# Publish a single crate (also useful to retry a failed one)
./scripts/publish_crates.py -p tari_engine --execute

# Resume after a failure
./scripts/publish_crates.py --from tari_engine_types --execute
```

The script skips crates whose current local version is already on crates.io,
so it is safe to re-run after a partial failure.

Prerequisites:
- `cargo login` (authenticate with crates.io).
- Versions are already set correctly in each `Cargo.toml` and pinned in the
  root `[workspace.dependencies]` block. Run `crate_versioning.py impact …`
  first to figure out which versions need to move.

---

## End-to-end recipe (the loop AI agents should follow)

When asked to bump a crate / cut a release:

1. **Decide the change kind.** Patch (non-breaking) or minor (breaking) — based
   on whether the public API changed shape.
2. **Run the impact analysis:**
   ```sh
   ./scripts/crate_versioning.py impact <crate> [--breaking]
   ```
3. **Apply the bumps the script printed** — and only those. Crates under
   "already covered by an unreleased bump" need nothing; bumping them anyway
   burns a version for no one's benefit. If the script says `No version changes
   required.`, skip to step 4.
   - If a tier-3 crate moved, update `[workspace.package].version` and every
     `version = "<old>"` pin in `[workspace.dependencies]` for tier-3 crates.
   - For each independent (non-core) crate the script listed, update its own
     `Cargo.toml` `version` (patch or minor as advised) and any pin(s) on the bumped deps.
   - A few pins live outside the root manifest (`ootle_ledger_client` in
     `crates/wallet/ootle-rs`, `ootle-wasm-core` in `crates/ootle_wasm/wasm`).
     `cargo metadata` fails loudly if one is left behind, so run it after editing.
4. **Format:**
   ```sh
   cargo +nightly-2025-12-05 fmt --all
   ```
5. **Dry-run the publish:**
   ```sh
   ./scripts/publish_crates.py --dry-run
   ```
6. **Commit, push, and publish for real after CI is green:**
   ```sh
   ./scripts/publish_crates.py --execute
   ```

If a publish step fails partway through, fix the issue and resume with
`--from <failed-crate> --execute`. Don't restart from the top — already-published
crates skip themselves automatically, but it's wasted CI time.

---

## Adding or removing a published crate

1. Add the `(name, path, tier)` tuple to `CRATES` in `publish_crates.py` in
   topological order (after its dependencies).
2. Add it to the publish set in `crate_versioning.py::PUBLISH_SET` **wait — no.**
   `crate_versioning.py` re-imports `CRATES` from `publish_crates.py`, so the
   set is derived automatically. Nothing else to update.
3. Run `./scripts/crate_versioning.py list` to confirm the new crate shows up
   with the expected version and tier.
4. Run `./scripts/publish_crates.py --dry-run` to confirm the build works.

---

## Tests

The version arithmetic behind the registry check — is this version spent, does
this bump break that pin — has its own tests. No network, no cargo, no pytest:

```sh
python3 scripts/test_crate_versioning.py
```
