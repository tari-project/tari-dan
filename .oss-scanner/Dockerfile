# Scanner image for https://github.com/tari-project/tari-ootle (development): the Tari Ootle validator node, indexer,
# wallet daemon and the crates under them.
#
# The audit runs offline, so this build fetches every crate the workspace and its standalone template crates lock, then
# compiles the workspace's libraries, binaries and test binaries with all features, as CI's test job does. It runs the
# engine and builtin-template tests, which compiles every WASM test template into its own target directory, so later
# template tests start warm. threat_model.md, next to this file, says how to run the rest.

# The Rust version must match the channel in rust-toolchain.toml, so that rustup installs nothing further.
FROM rust:1.97.1-trixie
ENV DEBIAN_FRONTEND=noninteractive

# The system libraries docker/ootle.Dockerfile links against: rocksdb and the wasmer/cranelift crates build C++ with
# clang and cmake, prost-build needs protoc, and the ledger client links hidapi, udev and dbus.
RUN apt-get update && apt-get install -y --no-install-recommends \
      clang cmake libc++-dev libc++abi-dev libdbus-1-dev libhidapi-dev libprotobuf-dev libreadline-dev \
      libsqlite3-dev libssl-dev libudev-dev pkg-config protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# tari_template_builtin's build.rs, and the template tests, compile templates with `cargo build --target
# wasm32-unknown-unknown`.
RUN rustup target add wasm32-unknown-unknown

ARG NEXTEST_VERSION=0.9.80
RUN curl -fsSL "https://get.nexte.st/${NEXTEST_VERSION}/linux" | tar -xzf - -C "${CARGO_HOME}/bin" \
    && cargo nextest --version

# scanner contract: the checkout lives inside the image, at /src
COPY . /src
WORKDIR /src

ENV RUST_BACKTRACE=1 \
    CARGO_TERM_COLOR=never

# The workspace, then every standalone crate (a Cargo.toml with its own [workspace]): the builtin and test templates,
# the bench templates and the cargo-fuzz crate. Their lockfiles are not committed, so this also writes them. The
# ledger app targets a device SDK this image does not have.
RUN cargo fetch --locked \
    && find . -name Cargo.toml -not -path ./Cargo.toml -not -path '*/target/*' -not -path '*/node_modules/*' \
         -not -path './crates/wallet/ledger/app/*' -exec grep -l '^\[workspace\]' {} + \
       | while read -r manifest; do cargo fetch --manifest-path "$manifest" || exit 1; done

# CI's test profile: release with opt-level 1 and overflow checks on, as validators run it. The applications' `ts`
# feature, which --all-features enables, skips their web UI builds. db-inspector has no such feature, and its build
# script cannot run pnpm here, so the dist directory its include_dir! embeds is created empty instead. integration_tests
# is left out: its cucumber suite needs a Minotari base node and wallet. A step that fails fails the build, and
# primary_contact gets the log.
RUN mkdir -p utilities/db_inspector/web_ui/dist \
    && cargo build --locked --workspace --exclude integration_tests --all-features --profile ci-build \
      --lib --bins --examples \
    && cargo nextest run --locked --workspace --exclude integration_tests --all-features --cargo-profile ci-build \
      --no-run

# The engine and builtin-template tests, which compile every WASM test template. Do not let a failure abort the
# build.
RUN timeout 900 cargo nextest run --locked --workspace --exclude integration_tests --all-features \
      --cargo-profile ci-build --no-fail-fast -E 'package(tari_engine) | package(tari_template_builtin)' \
    || echo "WARNING: cargo nextest reported failures"

# Everything is fetched. From here on cargo must not try to reach a registry.
ENV CARGO_NET_OFFLINE=true
