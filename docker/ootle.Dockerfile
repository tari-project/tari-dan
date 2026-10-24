# syntax=docker/dockerfile:1.7
#
# Tari Ootle images:
#   runtime (default target) - ghcr.io/tari-project/ootle: tari_validator_node and tari_indexer
#   walletd                  - ghcr.io/tari-project/ootle-walletd: tari_ootle_walletd
#
# See docs/docker-build-pipeline.md for an architecture overview.
#
# Stages:
#   sysdeps - rust toolchain + the system libraries every build links against
#   ci      - sysdeps, published as the image the CI jobs run inside
#   chef    - sysdeps + node + pnpm + cargo-chef
#   planner - emits cargo-chef recipe (dependency graph snapshot)
#   builder - cooks Rust deps, installs JS deps, builds binaries
#   base    - minimal Debian 13 with tini and the non-root user
#   walletd - base plus tari_ootle_walletd
#   runtime - base plus the network node binaries
#
# Build:
#   DOCKER_BUILDKIT=1 docker build -f docker/ootle.Dockerfile -t ootle:local .
#   DOCKER_BUILDKIT=1 docker build -f docker/ootle.Dockerfile --target walletd -t ootle-walletd:local .
#
# Run:
#   docker run --rm ootle:local tari_validator_node --help
#   docker run --rm ootle-walletd:local --help

ARG RUST_VERSION=1.97.1
ARG DEBIAN_VERSION=trixie
ARG NODE_MAJOR=24
ARG PNPM_VERSION=9


# ---------------------------------------------------------------------------
# sysdeps: rust toolchain and the system libraries every build links against
# ---------------------------------------------------------------------------
# Note: Docker Hub uses `<rust>-slim-<distro>` tag ordering (slim before distro).
FROM rust:${RUST_VERSION}-slim-${DEBIAN_VERSION} AS sysdeps

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HTTP_MULTIPLEXING=false

# System build dependencies. Trimmed subset of scripts/install_ubuntu_dependencies.sh
# - omits things only useful on dev workstations (zip, less, dh-autoreconf, ncurses).
# Includes:
#   - C/C++ toolchain (make, cmake, clang, g++, libc++) for native crates
#   - libssl, libsqlite3, libreadline (linked by Rust crates at build time)
#   - libudev, libhidapi, libdbus (for ledger HW wallet support in wallet_cli)
#   - protobuf-compiler (for prost-build)
#   - git (for cargo to fetch git deps)
#   - curl, ca-certificates, gnupg (bootstrap for NodeSource in the chef stage)
#   - mold, the linker the CI jobs select via `-fuse-ld=mold`
#   - sudo, so that CI actions which install to system paths work when a job
#     runs inside this image (see .github/workflows/ci_base_image.yml)
#
# If you add a binary that needs more system deps, add them here. The repo's
# scripts/install_ubuntu_dependencies.sh is the source of truth for dev setup;
# keep this list aligned with it.
#
# apt cache and lists are mounted so they persist across builds.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      clang \
      cmake \
      curl \
      dh-autoreconf \
      g++ \
      git \
      gnupg \
      libc++-dev \
      libc++abi-dev \
      libdbus-1-dev \
      libhidapi-dev \
      libncurses-dev \
      libprotobuf-dev \
      libreadline-dev \
      libsqlite3-dev \
      libssl-dev \
      libudev-dev \
      make \
      mold \
      openssl \
      pkg-config \
      protobuf-compiler \
      sudo

# RUST_VERSION must match the channel pinned in the repo's rust-toolchain.toml.
# When they agree, rustup resolves the workspace's toolchain to the one already
# in the base image and installs nothing further.
#
# Wasm32 target is required: tari_template_builtin compiles its WASM
# templates via build.rs (`cargo build --target wasm32-unknown-unknown`).
RUN rustup target add wasm32-unknown-unknown


# ---------------------------------------------------------------------------
# ci: the image the CI jobs run inside
# ---------------------------------------------------------------------------
# Deliberately stops at sysdeps. The CI jobs compile and test the Rust
# workspace; node, pnpm and cargo-chef are build-pipeline tools they never
# invoke, and every megabyte here is pulled again on every job because the
# runners are ephemeral.
FROM sysdeps AS ci

WORKDIR /base


# ---------------------------------------------------------------------------
# chef: sysdeps plus the JS toolchain, cargo-chef and wasm-pack, for the binary build
# ---------------------------------------------------------------------------
FROM sysdeps AS chef

ARG NODE_MAJOR
ARG PNPM_VERSION

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash - && \
    apt-get install -y --no-install-recommends nodejs && \
    corepack enable && \
    corepack prepare "pnpm@${PNPM_VERSION}" --activate

# cargo-chef for Rust dependency prewarming.
RUN cargo install cargo-chef --locked

# The indexer web UI builds the workspace's ootle-wasm crate with wasm-pack, to
# decode the values the indexer serves.
RUN cargo install wasm-pack --version 0.15.0 --locked

WORKDIR /base


# ---------------------------------------------------------------------------
# planner: extract dependency recipe from the workspace
# ---------------------------------------------------------------------------
FROM chef AS planner

COPY . .
RUN cargo chef prepare --recipe-path /recipe.json


# ---------------------------------------------------------------------------
# builder: cook deps, install JS deps, build binaries
# ---------------------------------------------------------------------------
FROM chef AS builder

# Phase 3a: cook Rust dependencies only. Workspace members are stubbed,
# so no application build.rs runs and pnpm is not needed yet. This layer
# is reused as long as Cargo.lock and feature selections are unchanged.
# Only the dependencies of the shipped binaries are cooked.
#
# /base/target is intentionally NOT a cache mount: cargo-chef relies on
# the compiled deps written to target/ to persist as a regular image
# layer so the next cargo build sees them. Cache mounts disappear after
# the RUN ends. The cargo registry/git mounts are still caches because
# they only hold downloaded crate sources, not build outputs.
COPY --from=planner /recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo chef cook --release --recipe-path recipe.json \
      --bin tari_validator_node \
      --bin tari_indexer \
      --bin tari_ootle_walletd

# Phase 3b: copy full source and warm JS dependencies.
COPY . .

# pnpm workspace (covers the indexer, validator_node and walletd web UIs plus
# shared packages: bindings, clients/javascript/*, applications/theming).
RUN --mount=type=cache,target=/root/.local/share/pnpm/store,sharing=locked \
    pnpm install --frozen-lockfile

# Pre-build shared workspace packages so leaf web UIs can import them.
# The indexer and validator_node build.rs scripts assume the shared dists
# already exist.
RUN pnpm --filter "@tari-project/ootle-ts-bindings" run build-dev \
 && pnpm --filter "@tari-project/ootle-web-ui-theming" run build \
 && pnpm --filter "@tari-project/wallet_jrpc_client" run build \
 && pnpm --filter "@tari-project/indexer-client" run build

# Phase 3c: build the binaries. Each daemon's build.rs invokes pnpm
# to compile its embedded web UI; node_modules and package manager
# caches from Phase 3b are reused. /base/target inherits the cooked
# dependencies as a layer from Phase 3a - cargo's incremental build
# only compiles the workspace members themselves.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/root/.local/share/pnpm/store,sharing=locked \
    --mount=type=cache,target=/root/.npm,sharing=locked \
    cargo build --release --locked \
      --bin tari_validator_node \
      --bin tari_indexer \
      --bin tari_ootle_walletd && \
    mkdir -p /out /out-walletd && \
    cp \
      target/release/tari_validator_node \
      target/release/tari_indexer \
      /out/ && \
    cp target/release/tari_ootle_walletd /out-walletd/


# ---------------------------------------------------------------------------
# base: minimal Debian 13 with tini and the non-root user
# ---------------------------------------------------------------------------
FROM debian:${DEBIAN_VERSION}-slim AS base

ARG VERSION=unknown
ARG RUST_VERSION
ARG DEBIAN_VERSION

ENV DEBIAN_FRONTEND=noninteractive

# Runtime libraries only - no -dev packages, no toolchain.
# - libsqlite3-0 linked by the node daemons
# - openssl + ca-certificates for TLS
# - tini as PID 1 init
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      tini \
      ca-certificates \
      openssl \
      libsqlite3-0 && \
    rm -rf /var/lib/apt/lists/*

# Non-root user.
RUN groupadd --gid 1000 tari && \
    useradd --create-home --no-log-init --shell /bin/bash \
      --home-dir /home/tari \
      --uid 1000 --gid 1000 tari && \
    mkdir -p /home/tari/data && \
    chown -R tari:tari /home/tari

# Metadata for runtime introspection (`docker inspect`, `env` inside container).
ENV OOTLE_VERSION=$VERSION \
    OOTLE_RUST_VERSION=$RUST_VERSION \
    OOTLE_DEBIAN_VERSION=$DEBIAN_VERSION \
    USER=tari

USER tari
WORKDIR /home/tari


# ---------------------------------------------------------------------------
# walletd: tari_ootle_walletd, configured through environment variables
# ---------------------------------------------------------------------------
FROM base AS walletd

USER root

# - libdbus-1-3, linked by the OS keyring store (unused in a container, where
#   OOTLE_WALLETD_KEYRING_PASSWORD replaces the keyring, but still loaded)
# - curl for the health check
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      libdbus-1-3 \
      curl && \
    rm -rf /var/lib/apt/lists/* && \
    mkdir -p /data && \
    chown tari:tari /data

COPY --from=builder /out-walletd/ /usr/local/bin/

# The wallet keeps its config, database and logs under /data/<network>/.
# XDG_DATA_HOME puts the default burn proof directory on the volume too.
# JSON-RPC stays on the binary's 127.0.0.1:5100 default unless JRPC_LISTEN_ON is set.
ENV TARI_BASE_DIR=/data \
    XDG_DATA_HOME=/data/share

USER tari
VOLUME ["/data"]
EXPOSE 5100

HEALTHCHECK --interval=30s --timeout=5s --start-period=60s \
  CMD listen="${JRPC_LISTEN_ON:-127.0.0.1:5100}"; curl -fsS "http://127.0.0.1:${listen##*:}/health" || exit 1

ENTRYPOINT ["/usr/bin/tini", "--", "tari_ootle_walletd"]


# ---------------------------------------------------------------------------
# runtime: the network node binaries. Last stage, so it is the default target.
# ---------------------------------------------------------------------------
FROM base AS runtime

COPY --from=builder /out/ /usr/local/bin/

# tini as PID 1 for proper signal handling and zombie reaping.
# No CMD: caller must specify which binary to run, e.g.
#   docker run --rm ghcr.io/tari-project/ootle tari_validator_node --help
ENTRYPOINT ["/usr/bin/tini", "--"]
