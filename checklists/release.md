# Release checklist

Every release, no exceptions. A breaking release runs [release-breaking.md](release-breaking.md)
alongside this one; a fix on top of an existing tag runs [hotfix.md](hotfix.md) instead.

Shared reference — what a tag sets off, the command index, recovery: [README.md](README.md).

---

## 1. Pre-flight (before the tag) ← the part that must not be skipped

- [ ] **Scope is frozen.** No merges into `development` other than the version bump PR until the
      tag is pushed — the tag names the tip of `development`. **(human)**
- [ ] **Version number decided.** Breaking changes → minor (`0.y` → `0.(y+1)`); otherwise patch.
      Also run [release-breaking.md](release-breaking.md) if anything there applies. **(human)**
- [ ] **CI is green** on the tip of `development`.
- [ ] **Changelog entry written** for the new version in `CHANGELOG.md`, `## [X.Y.Z]`. A breaking
      release also needs an `### ⚠️ Upgrade notes` block.
- [ ] **TypeScript bindings regenerated** if any `ts_rs`-exported Rust type changed, and
      `bindings/package.json` bumped. An unbumped package is skipped by the publish job without
      failing it, so the release would ship bindings that do not match the binaries.
- [ ] **Crate versions decided.** For each crate you changed:
      ```sh
      ./scripts/crate_versioning.py impact <crate> [--breaking]
      ```
      Apply only the bumps it prints. A tier-3 (core) break moves `[workspace.package].version` and
      every tier-3 pin in `[workspace.dependencies]`; independent crates move on their own.
- [ ] **`[workspace.package].version` is the version you are about to tag.**
- [ ] `cargo +nightly-2025-12-05 fmt --all` and `cargo metadata` both clean (metadata catches a pin
      left behind outside the root manifest).
- [ ] **Version bump PR merged**, titled `chore: release vX.Y.Z`.
- [ ] **The gate passes:**
      ```sh
      ./scripts/release_check.py            # add --package to also cargo-package what it can
      ```
      It must print `READY TO TAG`. It checks: clean tree on a release branch and in sync; the tag
      is free locally and on origin; the changelog entry exists; every crate changed since the last
      release carries a version crates.io does not hold; the publish order is topological and every
      pin resolves; npm packages that changed are bumped; and it flags a touched activation
      schedule.
- [ ] Read the warnings it printed. Each one is a decision, not noise. **(human)**

## 2. Cut the tag

- [ ] Tag the version bump PR's merge commit on `development` with a **signed, annotated** tag:
      ```sh
      git checkout development && git pull
      ./scripts/release_check.py           # again, on the commit you tag — it is the last gate
      git tag -s v0.42.0 -m "v0.42.0 — <one-line headline>"
      git push origin v0.42.0              # or upstream_mut, whichever remote is the canonical repo
      ```
- [ ] Note the short sha the tag points at. Every release asset embeds it, and it is how downstream
      repos select the right build.

## 3. Watch the builds (before publishing anything)

- [ ] ```sh
      ./scripts/release_status.py --watch
      ```
      It renders the draft's assets against the build matrices, the tag's workflow runs, npm,
      crates.io and every downstream SDK, and refuses to say "safe to publish" while a required
      artifact is missing.
- [ ] Every **required** binary platform present. A `best_effort` leg (windows-arm64) may
      be missing — decide whether you care. **(human)**
- [ ] A red "Build Matrix of Binaries" run means a required leg failed. The draft is still
      assembled from the legs that succeeded, so read the matrix rather than the run's colour
      alone. **(human)**
- [ ] All six **FFI platforms** present. ootle-go cannot ship a gap.
- [ ] **Re-run a failed leg** rather than publishing without it:
      - binaries: re-run the failed job from the Actions UI, or push a `build-bins-*` / `build-all-*`
        branch at the tag's commit;
      - FFI: re-run, or push a `build-ffi-*` branch;
      - either way the new artifacts attach to the same draft. Assets from a second commit then show
        up in `release_status.py` as `assets from other commits` — see [README.md](README.md).

## 4. Deploy to the testnet and verify

- [ ] Bump the image tag in the devops repo: `ansible/group_vars/ootle_esmeralda.yaml` →
      `tari_ootle_image_tag: v0.42.0`.
- [ ] `ansible-playbook ootle.yaml` (see that repo's README for the vault password and limits).
- [ ] **Verify the fleet actually came up on the new binary**, not just that ansible exited 0:
      validators producing blocks, indexer following, wallet daemon serving. **(human)**
- [ ] For a protocol activation, confirm every validator is on the new binary **before** the
      activation epoch — see [release-breaking.md](release-breaking.md).

## 5. Publish the release

Only once step 3 is complete and step 4 is healthy.

- [ ] ```sh
      gh release edit v0.42.0 --repo tari-project/tari-ootle --draft=false
      ```
- [ ] Add or tidy the release notes from the changelog entry. **(human)**
- [ ] Docs and indexer UI deploys fire on publish — confirm both went green, and that the docs
      download table lists the new version for every platform you expect.

## 6. Publish crates

- [ ] ```sh
      ./scripts/publish_crates.py            # read-only summary of what will publish
      ./scripts/publish_crates.py --execute
      ```
- [ ] Resume a partial failure with `--from <crate> --execute`; already-published crates skip
      themselves.
- [ ] `./scripts/crate_versioning.py list` shows everything `released`.

## 7. Update the downstream SDKs

`release_status.py` shows the current state of all three. Update the ones a consumer would notice —
in doubt, update all three; they are cheap.

- [ ] **ootle-go** (vendors the FFI static libs; needs the release assets, draft or not):
      ```sh
      cd ~/tari/ootle-go
      # once per platform, e.g.:
      scripts/vendor_release.sh --tari-platform linux-x86_64 --goos linux --goarch amd64 --tag v0.42.0
      scripts/gen_provenance.sh
      make test
      ```
      Then PR into `main` and tag. If the C ABI moved, `ExpectedABIVersion` in
      `internal/cffi/cffi.go` must move with it — see [release-breaking.md](release-breaking.md).
- [ ] **ootle.ts** (`@tari-project/ootle` and friends; waits for this release's npm publishes):
      bump the two catalog pins in `pnpm-workspace.yaml` — `@tari-project/ootle-ts-bindings` and
      `@tari-project/ootle-wasm` — run the checks, bump the workspace version, merge, then tag `v*`
      to publish.
- [ ] **ootle-py** (vendors the `@tari-project/ootle-wasm` blob; also waits for the npm publish):
      ```sh
      cd ~/tari/ootle-py
      make update-wasm WASM_VERSION=0.42.0     # or scripts/update_wasm.py --version 0.42.0
      just test                                 # regenerate fixtures if the wire changed
      ```
      Bump `pyproject.toml`'s version, PR, then tag `v*` — PyPI publishes on the tag via trusted
      publishing.
- [ ] **Minotari base layer** (only when this release published a breaking `tari_sidechain`): bump
      its pin in `tari-project/tari`'s root `Cargo.toml` and PR into `development`. A patch release
      is picked up by the existing pin.

## 8. Announce

- [ ] Post the upgrade notes where validator operators and app developers will see them, leading
      with anything that requires action before an epoch boundary. **(human)**
