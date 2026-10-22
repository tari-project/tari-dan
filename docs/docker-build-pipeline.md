# Docker Build Pipeline

Reference document for engineers working on the Tari Ootle Docker build.

## TL;DR

- One Dockerfile: `docker/ootle.Dockerfile`
- One workflow: `.github/workflows/build_dockers.yml`
- Two images from the same build:
  - `ghcr.io/<owner>/ootle` (target `runtime`) containing the network node binaries, `tari_validator_node` and `tari_indexer`
  - `ghcr.io/<owner>/ootle-walletd` (target `walletd`) containing `tari_ootle_walletd`; see `docker/README.md`
- Both images are signed by digest with cosign keyless signing
- Platform: `linux/amd64` only
- Registry: GitHub Container Registry (GHCR) only
- Caching: BuildKit cache mounts + registry cache (`ghcr.io/<owner>/ootle-buildcache`, `mode=max`)
- Runner: self-hosted `ubuntu-high-cpu`
- Rust deps prewarmed via [`cargo-chef`](https://github.com/LukeMathWalker/cargo-chef)

## Binaries shipped

| Binary | Purpose |
|---|---|
| `tari_indexer` | Substate indexer (JSON-RPC API + embedded web UI) |
| `tari_validator_node` | Validator node daemon (consensus + embedded web UI) |
| `tari_ootle_walletd` | Wallet daemon (JSON-RPC API + embedded web UI), `ootle-walletd` image only |

The wallet CLI, swarm and operations tools ship as release binaries, not in an image.

Both binaries embed a React web UI at compile time via the
`include_dir!` macro pointing at `web_ui/dist/`.
This means the Dockerfile must build the web UIs before `cargo build`,
otherwise the embedded asset directory is empty.

## Image layout

- Base: `debian:trixie-slim` (Debian 13)
- User: `tari` (uid 1000, gid 1000), non-root
- Workdir: `/home/tari`
- Binaries: `/usr/local/bin/`
- Entrypoint: `tini` as PID 1 (`["/usr/bin/tini", "--"]`)
- No default `CMD` — caller picks the binary

### Running

```bash
docker run --rm ghcr.io/tari-project/ootle:latest tari_validator_node --help
docker run --rm ghcr.io/tari-project/ootle:latest tari_indexer --help
```

`tini` is bundled (not relying on `docker run --init`) because Kubernetes
runtimes don't always inject an init, and we need predictable PID 1 behaviour
across deployment environments.

## Dockerfile architecture

Four stages. See `docker/ootle.Dockerfile` for the source of truth.

```
chef    ─┐                       (toolchain + system deps + node + pnpm + cargo-chef)
         ├── planner             (cargo chef prepare → recipe.json)
         └── builder             (cargo chef cook → pnpm install → npm ci → cargo build)
                                          │
                                          ▼
                                       runtime (debian-slim, non-root, tini)
```

### Stage 1: `chef`

Installs the build toolchain once so subsequent stages share the layer cache:

- System packages via apt: `make`, `cmake`, `clang`, `g++`, `libc++-dev`,
  `libc++abi-dev`, `pkg-config`, `protobuf-compiler`, `libprotobuf-dev`,
  `libssl-dev`, `openssl`, `libreadline-dev`, `libsqlite3-dev`,
  `libudev-dev`, `libhidapi-dev`, `libdbus-1-dev`, `git`, `curl`,
  `ca-certificates`, `gnupg`. This is a trimmed subset of
  `scripts/install_ubuntu_dependencies.sh` — keep them aligned when adding
  new native dependencies.
- Node.js 24 from NodeSource
- `corepack enable && corepack prepare pnpm@9 --activate`
  (pnpm 9 matches `lockfileVersion: '9.0'` in `pnpm-lock.yaml`)
- `rustup default stable && rustup target add wasm32-unknown-unknown`
  (see "Rustup toolchain quirk" below)
- `cargo install cargo-chef --locked`

### Rustup toolchain quirk

The `rust:<version>-slim-trixie` base image pre-installs Rust as a
versioned toolchain (e.g. `1.95.0-x86_64-unknown-linux-gnu`). The repo's
`rust-toolchain.toml` pins `channel = "stable"`, which rustup treats as
a **different** toolchain (`stable-x86_64-unknown-linux-gnu`) even when
"stable" currently happens to resolve to the same version. When cargo
runs inside the workspace, rustup auto-installs the stable toolchain on
first invocation - but any targets installed on the default toolchain
are not visible.

The Dockerfile resolves this by running `rustup default stable` in the
chef stage before installing the wasm32 target. This way every
toolchain operation (target add, cargo install, cargo build) hits the
same toolchain that workspace builds will use. Without this, you'd see
spurious "wasm32-unknown-unknown target may not be installed" errors
during `tari_template_builtin`'s `build.rs` even though `rustup target
list --installed` shows the target.

apt operations use BuildKit cache mounts on `/var/cache/apt` and
`/var/lib/apt/lists` with `sharing=locked` so concurrent builds don't trash
each other.

### Stage 2: `planner`

Runs `cargo chef prepare --recipe-path /recipe.json` against a `COPY . .` of
the workspace. This produces a minimal "recipe" describing the dependency
graph (crate names + versions + features). The recipe is what gets cached;
the full source tree at this stage is discarded after extraction.

### Stage 3: `builder`

Three phases inside this stage. Each phase exists so its inputs are
cache-key boundaries — touching only files that affect a later phase keeps
earlier phase caches warm.

**Phase 3a — Cook Rust deps:**
```dockerfile
COPY --from=planner /recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json
```

`cargo chef cook` compiles **only third-party crates**, stubbing out
workspace members. So no `build.rs` for our apps runs here — pnpm doesn't
need to work yet.

**Important**: `/base/target` is NOT a BuildKit cache mount. cargo-chef
relies on the compiled deps written to `target/` persisting as a regular
image layer so that the next `cargo build` step sees them. BuildKit
cache mounts disappear when the RUN exits, which would defeat the whole
purpose. The `/usr/local/cargo/{registry,git}` mounts are caches because
they only hold downloaded crate sources, not build outputs.

This layer's cache key is `recipe.json`. The recipe only changes when
`Cargo.lock` or feature selections change — not when application source
changes. So most Rust source edits keep this expensive layer cached.
A measured incremental build (one Rust file changed) takes ~2 minutes
total wall-clock; a full cold build takes ~20-25 minutes.

**Phase 3b — JS dependencies:**
```dockerfile
COPY . .
RUN --mount=type=cache,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile
```

The repo has a pnpm workspace at root (`pnpm-workspace.yaml`) covering all
3 daemon web UIs plus shared packages (`bindings`, `clients/javascript/*`,
`applications/theming`). One `pnpm install` warms them all.

The install uses a cache mount for the pnpm store, so the network fetch
happens once across all builds on a given runner.

**Phase 3c — Build binaries:**
```dockerfile
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/base/target \
    --mount=type=cache,target=/root/.local/share/pnpm/store \
    --mount=type=cache,target=/root/.npm \
    cargo build --release --locked \
      --bin tari_validator_node \
      --bin tari_indexer \
    && mkdir -p /out \
    && cp target/release/{tari_validator_node,tari_indexer} /out/
```

Each daemon's `build.rs` invokes `pnpm` to compile its web UI into
`web_ui/dist/`. Because node_modules were warmed in Phase 3b and the pnpm
cache is mounted here, web UI builds are fast.

**Why copy to `/out/` inside the same RUN:** the binaries get extracted
to `/out/` so the runtime stage can grab them via a single
`COPY --from=builder /out/` rather than mirroring the cargo target
directory structure. This also keeps the path explicit and stable across
cargo version changes.

### Stage 4: `runtime`

Minimal Debian 13. Installs only runtime libraries:

- `tini` (PID 1 init)
- `ca-certificates`, `openssl` (TLS)
- `libsqlite3-0` (linked at runtime by both daemons)

Creates `tari` user (uid/gid 1000), copies the 2 binaries from `/out/`,
drops to `USER tari`, sets `WORKDIR /home/tari`, and sets
`ENTRYPOINT ["/usr/bin/tini", "--"]`.

No `CMD` is set. Callers must specify which binary to run:

```bash
docker run ghcr.io/tari-project/ootle:latest tari_validator_node [args...]
```

## Why these choices

### Why cargo-chef?

Naive Docker layering for Rust is poor because `COPY . . && cargo build`
makes every source change invalidate the dependency cache, forcing a full
recompile of every third-party crate (hundreds of crates, several minutes
each in a workspace this size).

`cargo-chef` decouples dependency compilation from source compilation:

1. `cargo chef prepare` emits a recipe (a stripped Cargo manifest).
2. `cargo chef cook` builds only the deps from the recipe (in a stage
   that has no app source).
3. Application code is copied in afterwards and `cargo build` reuses the
   cached `/base/target` to skip the deps.

The recipe rarely changes (only on `Cargo.lock` updates or feature
toggles), so the "cook" layer stays cached across most PRs.

### Why BuildKit cache mounts?

Cache mounts (`--mount=type=cache,...`) provide persistent storage to a
`RUN` step without baking the contents into the resulting image layer.
Three reasons we use them everywhere:

1. **Smaller image layers**: Cargo's `target/` directory can be 5-15 GB
   for a project this size. Baking it into a layer would bloat the image
   and the registry cache.
2. **Cross-build reuse**: When the layer cache is invalidated (e.g.
   source changed), the cache mount survives and cargo's incremental
   compilation reuses object files.
3. **Pnpm/npm store sharing**: The pnpm content-addressable store
   deduplicates across installs. A cache mount makes that work
   across builds.

The cache mount data is stored locally on the builder daemon. In CI it
persists across job runs because we set
`cache-to: type=gha,mode=max` — BuildKit serialises the cache mount
contents and ships them to GitHub Actions cache.

### Why GHA cache (`type=gha`) and not registry cache?

- GHA cache is free and built into the runner.
- 10 GB/repo cap with LRU eviction; we accept this trade-off initially.
- Registry cache (`type=registry,ref=ghcr.io/.../buildcache,mode=max`)
  is the next step if eviction becomes a problem — switch is one line.

### Why amd64-only?

We had cross-compile machinery for arm64 that was unused on the actual
arm64 runner (`ubuntu-24.04-arm`), and the per-arch matrix doubled CI
time without adding value for the current deployment targets. Adding
arm64 later means:

1. Restore matrix in the workflow with two runners.
2. Use `docker buildx imagetools create` to produce a multi-arch
   manifest list (one extra job, ~30 lines).

### Why Node 24 + pnpm 9?

- Node 24 is the current Node.js LTS (as of 2025).
- pnpm 9 matches `lockfileVersion: '9.0'` in `pnpm-lock.yaml`. If you
  upgrade pnpm, regenerate the lockfile.
- Corepack is the supported Node-bundled mechanism for shipping a
  specific pnpm version. Note: corepack is marked deprecated in Node
  upstream; if it's removed, switch to `npm install -g pnpm@9` in the
  chef stage.

### Why tini and not dumb-init?

Both work. `tini` is what `docker run --init` uses under the hood, is
actively maintained, and is a one-line apt install on Debian. Bundling
it (vs. relying on `docker run --init`) matters because Kubernetes
runtimes don't always inject an init process, and we want predictable
zombie-reaping + signal forwarding in every deployment.

### Why no `CMD`?

The image ships 2 binaries with no canonical entrypoint. Picking one as
the default would mislead users of the others. Making the binary part
of `docker run` is explicit.

## GitHub Actions workflow

Located at `.github/workflows/build_dockers.yml`. ~80 lines total.

### Triggers

- `push` of tags matching `v[0-9]+.[0-9]+.[0-9]*`
- `workflow_dispatch` (manual)
- `schedule`: nightly at 00:05 UTC, Sun-Fri

### Tags produced

Driven by `docker/metadata-action`:

| Source | Tag |
|---|---|
| Any run | `sha-<short>` |
| Manual/scheduled run on a branch | `<branch-name>` |
| Tag `vX.Y.Z` | `vX.Y.Z`, `X.Y.Z`, `X.Y`, `latest` |
| Schedule (nightly) | `nightly` |

All push to `ghcr.io/<owner>/ootle`.

### Authentication

Uses the auto-provided `GITHUB_TOKEN` to push to GHCR. No external
secrets required. The repository must have:
- Settings → Actions → Workflow permissions: "Read and write permissions"
- Settings → Packages: the `ootle` package linked to this repo
  (auto-links on first push)

### Concurrency

```yaml
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: ${{ !startsWith(github.ref, 'refs/tags/v') }}
```

Cancels superseded runs on adhoc branches. Never cancels release tag
builds.

## Local development

### Build the image

```bash
DOCKER_BUILDKIT=1 docker build \
  -f docker/ootle.Dockerfile \
  -t ootle:local .
```

First build takes ~15-25 min depending on hardware. Subsequent builds
should be 30 seconds to 5 minutes depending on what changed.

### Run a binary

```bash
docker run --rm ootle:local tari_validator_node --version
docker run --rm ootle:local tari_indexer --help
```

### Inspect the image

```bash
docker run --rm ootle:local ls -la /usr/local/bin/
docker run --rm ootle:local id     # should report uid=1000(tari)
```

### Debug a failed build

BuildKit hides intermediate stage output by default. To see it:

```bash
DOCKER_BUILDKIT=1 docker build \
  --progress=plain \
  -f docker/ootle.Dockerfile \
  -t ootle:local .
```

To inspect a stage without running the whole build:

```bash
DOCKER_BUILDKIT=1 docker build \
  --target builder \
  -f docker/ootle.Dockerfile \
  -t ootle-builder:local .
docker run --rm -it ootle-builder:local bash
```

Valid `--target` values: `chef`, `planner`, `builder`, `base`, `walletd`, `runtime`.

## Common changes

### Add a binary to the image

1. Add `--bin <name>` to the `cargo build` invocation in
   `docker/ootle.Dockerfile` (Phase 3c).
2. Add the path to the `cp target/release/{...} /out/` list.
3. Update the binary table in this document.

### Change Rust version

The image base is `rust:1.95-slim-trixie`. Bump the tag in the `chef`
stage `FROM` line. The actual toolchain inside the build is determined
by `rust-toolchain.toml` (currently `channel = "stable"`), so the base
image version just controls what's pre-installed before rustup
potentially upgrades.

### Change Node version

Update the NodeSource setup URL in the `chef` stage:
`curl -fsSL https://deb.nodesource.com/setup_<N>.x | bash -`.

### Change pnpm version

Update `corepack prepare pnpm@<version> --activate` in the chef stage.
Must be compatible with `lockfileVersion` in `pnpm-lock.yaml` — bumping
across pnpm majors usually requires lockfile regeneration.

### Add arm64

1. Add `linux/arm64` to `platforms:` in the workflow's `build-push-action`.
2. Decide between (a) QEMU emulation (slow, single job) or (b) matrix
   with an arm64 runner (`ubuntu-24.04-arm`) + `buildx imagetools create`
   to merge manifests.
3. Update `cache-from`/`cache-to` to use per-arch refs to avoid cache
   contention.

### Switch from GHA cache to registry cache

Replace in the workflow:

```yaml
cache-from: type=gha
cache-to: type=gha,mode=max
```

with:

```yaml
cache-from: type=registry,ref=ghcr.io/${{ github.repository_owner }}/ootle-buildcache
cache-to: type=registry,ref=ghcr.io/${{ github.repository_owner }}/ootle-buildcache,mode=max
```

The buildcache image will be created automatically on first push.
Recommend setting it to private in GHCR settings.

## Troubleshooting

### `cargo chef cook` fails with linker errors

Usually means a system dep is missing. Add it to the apt install in the
chef stage. Common culprits: `libpq-dev` (postgres), `libzmq3-dev`,
`libgmp-dev`.

### `pnpm install` fails with `ERR_PNPM_LOCKFILE_BREAKING_CHANGE`

The lockfile was generated by a newer pnpm than the one in the image.
Either:
- Bump `corepack prepare pnpm@<newer> --activate` in the chef stage, or
- Regenerate the lockfile with `pnpm install` locally on the older version.

### Web UI not showing up in the running container

The web UI is embedded at compile time. If `web_ui/dist/` was empty when
`cargo build` ran, the binary will serve nothing. Check the build log
for `cargo:warning=The web ui will not be included` lines from the
relevant `build.rs`.

### Image is much larger than expected

Run `docker history ootle:local` to find the biggest layer. The runtime
image should be under 200 MB. If it's much larger:
- Cargo target accidentally baked into a layer (cache mount missing or
  wrong path) — check the Dockerfile cache mounts cover all cargo paths.
- A `COPY --from=builder` brought too much — should only copy `/out/`.

### GHA build hits the 10 GB cache cap

Symptoms: cache-from misses on every run despite no source change.
Switch to registry cache (see above).

## File reference

| Path | Purpose |
|---|---|
| `docker/ootle.Dockerfile` | The Dockerfile |
| `docker/README.md` | Short user-facing build instructions |
| `docs/docker-build-pipeline.md` | This document (engineer reference) |
| `.github/workflows/build_dockers.yml` | CI workflow |
| `.dockerignore` | Excludes from the build context |
| `pnpm-workspace.yaml` | Defines which JS packages share node_modules |
| `pnpm-lock.yaml` | Pinned JS dep tree (root workspace) |
| `rust-toolchain.toml` | Rust channel pin (currently `stable`) |

## History

- Originally a multi-arch (amd64 + arm64) build with cross-compile
  tooling, per-arch matrix runners, manual `docker manifest create`,
  dual-registry push (GHCR + Quay), reusable workflow plus orchestrator
  workflow, and one monolithic `RUN` step that defeated layer caching.
- Simplified to amd64-only, single-registry (GHCR), single workflow,
  single Dockerfile, with cargo-chef + BuildKit cache mounts for
  fast incremental builds.
- See git history of `docker/`, `.github/workflows/build_dockers*`, and
  `buildtools/docker_rig/` for the prior implementation.
