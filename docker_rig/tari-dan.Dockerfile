# syntax = docker/dockerfile:1.3

# https://hub.docker.com/_/rust
ARG RUST_VERSION=1.74
ARG OS_BASE=bookworm

# Node Version
ARG NODE_MAJOR=20

# rust source compile with cross platform build support
FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-${OS_BASE} as builder-tari-dan

# Declare to make available
ARG BUILDPLATFORM
ARG BUILDOS
ARG BUILDARCH
ARG BUILDVARIANT
ARG TARGETPLATFORM
ARG TARGETOS
ARG TARGETARCH
ARG TARGETVARIANT
ARG RUST_TOOLCHAIN
ARG RUST_TARGET
ARG RUST_VERSION
ARG OS_BASE

# Node Version
ARG NODE_MAJOR
ENV NODE_MAJOR=$NODE_MAJOR

# Prep nodejs lts - 20.x
RUN apt-get update && apt-get install -y \
      apt-transport-https \
      ca-certificates \
      curl \
      gpg && \
      mkdir -p /etc/apt/keyrings && \
      curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg && \
      echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_${NODE_MAJOR}.x nodistro main" | tee /etc/apt/sources.list.d/nodesource.list

RUN apt-get update && apt-get install -y \
      libreadline-dev \
      libsqlite3-0 \
      openssl \
      cmake \
      protobuf-compiler \
      nodejs

# https://gcc.gnu.org/onlinedocs/gcc/x86-Options.html
ARG ARCH
#ARG ARCH=native

#ENV RUSTFLAGS="-C target_cpu=$ARCH"
#ENV ROARING_ARCH=$ARCH
ENV CARGO_HTTP_MULTIPLEXING=false

WORKDIR /tari-dan

ADD . .

RUN if [ "${BUILDARCH}" != "${TARGETARCH}" ] ; then \
      # Run script to help setup cross-compile environment
      . /tari-dan/docker_rig/cross-compile-tooling.sh ; \
    fi && \
    if [ -n "${RUST_TOOLCHAIN}" ] ; then \
      # Install a non-standard toolchain if it has been requested.
      # By default we use the toolchain specified in rust-toolchain.toml
      rustup toolchain install ${RUST_TOOLCHAIN} --force-non-host ; \
    fi && \
    cd /tari-dan/applications/tari_indexer_web_ui && \
    npm install react-scripts && \
    npm run build && \
    cd /tari-dan/applications/tari_validator_node_web_ui && \
    npm install react-scripts && \
    npm run build && \
    cd /tari-dan/ && \
    rustup target add wasm32-unknown-unknown && \
    rustup target list --installed && \
    rustup toolchain list && \
    rustup show && \
    cargo build ${RUST_TARGET} \
      --release --locked \
      --bin tari_indexer \
      --bin tari_dan_wallet_daemon \
      --bin tari_dan_wallet_cli \
      --bin tari_signaling_server \
      --bin tari_validator_node \
      --bin tari_validator_node_cli && \
    # Copy executable out of the cache so it is available in the runtime image.
    cp -v /tari-dan/target/${BUILD_TARGET}release/tari_indexer /usr/local/bin/ && \
    cp -v /tari-dan/target/${BUILD_TARGET}release/tari_dan_wallet_daemon /usr/local/bin/ && \
    cp -v /tari-dan/target/${BUILD_TARGET}release/tari_dan_wallet_cli /usr/local/bin/ && \
    cp -v /tari-dan/target/${BUILD_TARGET}release/tari_signaling_server /usr/local/bin/ && \
    cp -v /tari-dan/target/${BUILD_TARGET}release/tari_validator_node /usr/local/bin/ && \
    cp -v /tari-dan/target/${BUILD_TARGET}release/tari_validator_node_cli /usr/local/bin/ && \
    echo "Tari Dan Build Done"

# Create runtime base minimal image for the target platform executables
FROM --platform=$TARGETPLATFORM debian:${OS_BASE} as runtime

ARG BUILDPLATFORM
ARG TARGETPLATFORM
ARG TARGETOS
ARG TARGETARCH
ARG TARGETVARIANT
ARG RUST_VERSION
ARG OS_BASE

ARG VERSION

# Disable Prompt During Packages Installation
ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get --no-install-recommends install -y \
      dumb-init \
      ca-certificates \
      openssl

RUN groupadd --gid 1000 tari && \
    useradd --create-home --no-log-init --shell /bin/bash \
      --home-dir /home/tari \
      --uid 1000 --gid 1000 tari

ENV dockerfile_target_platform=$TARGETPLATFORM
ENV dockerfile_version=$VERSION
ENV dockerfile_build_platform=$BUILDPLATFORM
ENV rust_version=$RUST_VERSION

# Setup some folder structure
RUN mkdir -p "/home/tari/data" && \
    chown -R tari:tari "/home/tari/"

COPY --chown=tari:tari --from=builder-tari-dan /usr/local/bin/tari_* /usr/local/bin/

WORKDIR /home/tari
ENV USER=tari
#CMD [ "tail", "-f", "/dev/null" ]
#CMD ["dumb-init", "node", "./bin/www"]
