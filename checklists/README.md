# Checklists

Read-and-do procedures, written to be run top to bottom by a human or an AI agent. Every item is
either a command whose output decides the next step, or a judgement only a person can make — marked
**(human)**. Nothing here is a summary of how things generally work; if an item cannot be acted on
as written, it is a bug in the checklist.

| Checklist | When |
|---|---|
| [release.md](release.md) | every release, no exceptions |
| [release-breaking.md](release-breaking.md) | in addition, when consensus, the engine, an on-chain ABI, a config struct or an SDK surface changes |
| [hotfix.md](hotfix.md) | a fix on top of an already-tagged version |

Shared reference for all three is below.

---

## What a tag sets off

Pushing `v<major>.<minor>.<patch>` starts six independent pipelines. None of them knows about the
others, and **a failed leg does not fail the release** — several build legs are `best_effort`, and
the npm jobs silently skip a package whose version is already published.

| Trigger | Fires | Produces | Reversible? |
|---|---|---|---|
| tag push | `build_binaries.yml` | node + wallet binaries → **draft** release | yes, until published |
| tag push | `ffi_libs.yml` | `ootle_sdk_ffi_c` libs per platform → same draft | yes |
| tag push | `build_dockers.yml` | `ootle` and `ootle-walletd` images, tagged `v…`, `<maj>.<min>`, `latest` (pre-release tags skip `<maj>.<min>` and `latest`) | **no** |
| tag push | `npm_publish.yml` | 3 npm packages | **no** |
| tag push | `npm_publish_ootle_wasm.yml` | `@tari-project/ootle-wasm` | **no** |
| **release published** | `docs-deploy.yml`, `indexer-web-ui-deploy.yml` | developer docs + indexer UI to Cloudflare | redeployable |

Two consequences that drive every checklist here:

1. **Publishing the draft is the public moment, not the tag.** The developer docs render the wallet
   downloads of the newest *non-draft* release, so publishing a draft with a platform missing gives
   that platform's users a download table with nothing in it. v0.41.0 went out without Windows
   wallet binaries exactly this way.
2. **crates.io, npm, PyPI and docker tags are immutable.** Everything that can be checked before the
   tag is checked before the tag — that is what `release_check.py` is for.

---

## Command index

```sh
# Pre-tag gate — must print READY TO TAG
./scripts/release_check.py [--since <tag>] [--offline] [--package]

# Post-tag dashboard — draft assets, workflow runs, npm, crates.io, downstream SDKs
./scripts/release_status.py [<tag>] [--watch] [--interval 120]

# Version arithmetic
./scripts/crate_versioning.py list
./scripts/crate_versioning.py impact <crate> [--breaking]

# Publishing to crates.io
./scripts/publish_crates.py [--dry-run | --execute | --from <crate> --execute]

# Publish the draft (fires the docs + indexer UI deploys)
gh release edit <tag> --repo tari-project/tari-ootle --draft=false
```

Deeper reference for the versioning model and tiers: [`scripts/README.md`](../scripts/README.md).

> `cargo publish --dry-run` routinely fails during a release and that failure means nothing: a dry
> run resolves every dependency against the registry, so any crate in this release whose new version
> has not been published *yet* cannot resolve. `release_check.py`'s publish preflight answers the
> same question honestly — it resolves pins against crates.io **plus** what this release publishes.

---

## If something goes wrong

**Immutable once published** — plan around them, they cannot be taken back: crates.io versions
(yank only hides them from new resolutions), npm and PyPI versions, docker tags, a published GitHub
release.

**Still reversible**: a draft release (delete it), an unpushed tag, a tag nobody has consumed
(`git push --delete origin v0.42.0` — only before the publish jobs have run, and never once a
crates.io/npm/PyPI artifact carries that version).

Common situations:

- **A build leg failed.** Re-run it; the artifacts attach to the same draft. Do not publish around
  it — see the Windows binaries in v0.41.0.
- **The release line was re-cut and the draft holds assets from two commits.** `release_status.py`
  prints `assets from other commits`. Delete the stale assets, or make sure consumers select by
  commit: ootle-go's `vendor_release.sh` already pins to the tag's own commit and takes
  `--commit <sha>` to override.
- **A crate published with a wrong version.** The number is spent. Publish a new patch; yank only if
  the bad version is actively harmful.
- **An npm package did not publish.** Almost always an unbumped version — the job skips it silently.
  Bump and cut a patch tag; there is no way to republish a version.
- **The testnet is unhealthy after deploy.** Do not publish the release. Roll the image tag back in
  ansible, redeploy, and fix forward with a [hotfix](hotfix.md).

---

## Branches

`development` is the only long-lived branch. PRs land on it and a release is tagged from its tip,
so every tag is an ancestor of the next one and `release_check.py` needs no `--since` in the normal
case. A hotfix is the exception: it branches from the tag it patches, is tagged on that `hotfix/*`
branch, and reaches `development` afterwards. `main` is no longer merged into or tagged from.

`RELEASE_BRANCHES` in `scripts/release_check.py` holds the branches a release may be cut from;
`hotfix/*` branches are accepted alongside it.
