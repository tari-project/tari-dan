# Tari Ootle - Claude Code Cheatsheet

## Project Structure

- The project is called "Ootle" or "Tari Ootle" (not "DAN" or "Tari DAN")
- **Crates** are in `crates/`, applications in `applications/`, clients in `clients/`
- Wallet daemon package: `tari_ootle_walletd` (in `applications/tari_walletd/`)
- Wallet CLI: `tari_wallet_cli` (in `applications/tari_wallet_cli/`)
- Transaction manifest parser: `tari_transaction_manifest` (in `crates/transaction_manifest/`)
- Transaction types/instructions: `tari_ootle_transaction` (in `crates/transaction/`)
- Template lib types (OwnerRule, RistrettoPublicKeyBytes, etc.): `tari_template_lib_types` (in
  `crates/template_lib_types/`)
- CBOR serialization: `tari_bor` (in `crates/tari_bor/`) - wraps ciborium, re-exports serde

## Code Comments

IMPORTANT: Comments must help a future reader understand code that isn't self-explanatory — non-obvious
invariants, the reason for a non-obvious approach, tricky edge cases. They must NOT narrate changes.

- When fixing a bug, lint, or doing a refactor, do not add a comment explaining what was wrong or why you
  changed it. The corrected code no longer has the issue, so such a comment describes a problem that isn't
  in the code — it serves no future reader.
- Rationale for a fix belongs in the commit message and PR description, not in the source.
- Never answer a reviewer in a code comment. When addressing review feedback, the justification goes in the
  reply on the review thread; the source gets only what a reader who never saw the review needs. A comment
  that argues for the new code over what the reviewer objected to is narrating a change — the thing it
  contrasts against isn't in the file.
- Write present-tense statements of intent/invariant ("X must be Y because Z"), never history ("changed
  this from A to B", "without this guard X would happen", "previously this did A").

## Key Facts

### Token Amounts

- All amounts in code/manifests are in **microtari**: 1 TARI = 1,000,000 microtari
- In manifests, use `TARI` as the resource identifier (not `XTR`, which is deprecated)
- User-facing: use **tTARI** (testnet) or **$TARI** (mainnet)

### Transaction Manifests

- DSL parsed by `tari_transaction_manifest::parse_manifest()`
- Parser is in `crates/transaction_manifest/src/parser.rs`, generator in `generator.rs`
- Macros: `var!`/`arg!`/`global!`, `new_component_addr!`, `new_resource_addr!`, `create_account!`,
  `drop_all_proofs!`
- `create_account!(owner_pk)` generates `Instruction::CreateAccount` - idempotent account creation
- Variables passed to manifests are parsed via `ManifestValue::FromStr` which handles: substate IDs (`component_<hex>`,
  `resource_<hex>`, etc.), NFT IDs, Rust literals, and raw hex bytes (for public keys)

### Wallet Daemon JSON-RPC

- Default port: 5100, endpoint: `/json_rpc`, ask the user if the daemon is running on a different port
- Auth: check `auth.method`, then `auth.request` with `credentials: "None"` for no-auth setups
- JWT tokens expire after 5 minutes
- Key endpoints: `accounts.get_default`, `accounts.list`, `transactions.submit_manifest`, `transactions.wait_result`
- `transactions.submit_manifest` variables are strings parsed by `ManifestValue::FromStr`

### PR Workflow

- Base branch for PRs is `development`, not `main`

### Building

- Standard cargo workspace. Use `-p <package_name>` to build/test specific crates

### Formatting

Use `cargo  +nightly-2025-12-05 fmt --all` to format all code with the specified nightly version before pushing commits
involving rust code.

### Publishing & Crate Versioning

Two scripts in `scripts/` cover publishing to crates.io and reasoning about version bumps. Use them — don't hand-walk
`Cargo.toml`s.

- `scripts/publish_crates.py` — publishes the workspace's public crates to crates.io in topological order.
  - `--list` shows the publish order with version + tier.
  - Default (no flags) is a dry-run summary. `--dry-run` runs `cargo publish --dry-run` per crate. `--execute` publishes.
  - `--from <crate>` resumes after a failure.
- `scripts/crate_versioning.py` — answers "who needs to bump if X bumps?".
  - `list` — same publish set with versions/tiers, plus whether each version is already on crates.io (`released`) or
    still unpublished (`pending`).
  - `deps <crate>` / `dependents <crate> [--transitive]` — graph queries.
  - `impact <crate> [--breaking]` — for a non-breaking change, prints just a patch bump. For `--breaking`, prints the
    tier-3 workspace rollup (every tier-3 crate moves with `workspace.package.version`) plus the tier-1/2 crates that
    need pin updates + their republish guidance (patch min, minor if their public API re-exposes the changed types).
  - Both commands query crates.io and ask only for bumps that are still outstanding. A crate whose in-tree version is
    an unreleased minor ahead of crates.io needs nothing — the pending release already carries the change, because a
    `^0.y` pin cannot exist on a version that was never published. `--offline` skips the check and lists the whole
    cascade instead.

When asked to bump a crate or prepare a release, run `impact <crate> --breaking` (or without `--breaking` for a patch)
first, then follow its workflow checklist — and apply only the bumps it asks for, not the ones it lists as already
covered. Both scripts share their crate list — `publish_crates.py::CRATES` is the source of truth, so adding/removing a
published crate there automatically updates `crate_versioning.py`. Detailed reference: `scripts/README.md`.
