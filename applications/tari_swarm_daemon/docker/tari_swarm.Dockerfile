# syntax = docker/dockerfile:1.3

# https://hub.docker.com/_/rust
ARG RUST_VERSION=1.76
ARG RUST_DISTRO=bookworm

# Node Version
ARG NODE_MAJOR=20

# rust source compile with cross platform build support
FROM --platform=$BUILDPLATFORM rust:$RUST_VERSION-$RUST_DISTRO as builder-tari

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
ARG RUST_DISTRO

ARG DAN_TESTING_WEBUI_PORT=18000

# Node Version
ARG NODE_MAJOR
ENV NODE_MAJOR=$NODE_MAJOR

# Prep nodejs lts - 20.x
# https://github.com/nodesource/distributions
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
      cargo \
      clang \
      gcc-aarch64-linux-gnu \
      g++-aarch64-linux-gnu \
      cmake \
      nodejs

ARG ARCH=native
#ARG FEATURES=avx2
ARG FEATURES=safe
ENV RUSTFLAGS="-C target_cpu=$ARCH"
ENV ROARING_ARCH=$ARCH
ENV CARGO_HTTP_MULTIPLEXING=false

ARG VERSION=0.0.1

RUN if [ "${BUILDARCH}" != "${TARGETARCH}" ] && [ "${ARCH}" = "native" ] ; then \
      echo "!! Cross-compile and native ARCH not a good idea !! " ; \
      fi



ADD sources/tari /home/tari/sources/tari

WORKDIR /home/tari/sources/tari

RUN ./scripts/install_ubuntu_dependencies.sh
RUN if [ "${TARGETARCH}" = "arm64" ] && [ "${BUILDARCH}" != "${TARGETARCH}" ] ; then \
      # Hardcoded ARM64 envs for cross-compiling - FixMe soon
      # source /tari/cross-compile-aarch64.sh
      . ../cross-compile-aarch64.sh ; \
      fi && \
      if [ -n "${RUST_TOOLCHAIN}" ] ; then \
      # Install a non-standard toolchain if it has been requested.
      # By default we use the toolchain specified in rust-toolchain.toml
      rustup toolchain install ${RUST_TOOLCHAIN} --force-non-host ; \
      fi && \
      rustup target list --installed && \
      rustup toolchain list && \
      rustup show && \
      cargo build ${RUST_TARGET} \
      --release --features ${FEATURES} --locked \
      --bin minotari_node \
      --bin minotari_console_wallet \
      --bin minotari_miner && \
      # Copy executable out of the cache so it is available in the runtime image.
      cp -v ./target/${BUILD_TARGET}release/minotari_node /usr/local/bin/ && \
      cp -v ./target/${BUILD_TARGET}release/minotari_console_wallet /usr/local/bin/ && \
      cp -v ./target/${BUILD_TARGET}release/minotari_miner /usr/local/bin/minotari_sha && \
      echo "Tari Build Done"

# rust source compile with cross platform build support
FROM --platform=$BUILDPLATFORM rust:$RUST_VERSION-$RUST_DISTRO as builder-tari-dan

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
ARG RUST_DISTRO

# Node Version
ARG NODE_MAJOR
ENV NODE_MAJOR=$NODE_MAJOR

# Prep nodejs lts - 20.x
# https://github.com/nodesource/distributions
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
      cargo \
      clang \
      gcc-aarch64-linux-gnu \
      g++-aarch64-linux-gnu \
      cmake \
      nodejs

ARG ARCH=native
ENV RUSTFLAGS="-C target_cpu=$ARCH"
ENV ROARING_ARCH=$ARCH
ENV CARGO_HTTP_MULTIPLEXING=false

RUN if [ "${BUILDARCH}" != "${TARGETARCH}" ] && [ "${ARCH}" = "native" ] ; then \
      echo "!! Cross-compile and native ARCH not a good idea !! " ; \
      fi

ADD sources/tari-dan /home/tari/sources/tari-dan

WORKDIR /home/tari/sources/tari-dan
RUN ./scripts/install_ubuntu_dependencies.sh
RUN if [ "${TARGETARCH}" = "arm64" ] && [ "${BUILDARCH}" != "${TARGETARCH}" ] ; then \
      # Hardcoded ARM64 envs for cross-compiling - FixMe soon
      # source /tari-dan/cross-compile-aarch64.sh
      . ../cross-compile-aarch64.sh ; \
      fi

RUN if [ -n "${RUST_TOOLCHAIN}" ] ; then \
      # Install a non-standard toolchain if it has been requested.
      # By default we use the toolchain specified in rust-toolchain.toml
      rustup toolchain install ${RUST_TOOLCHAIN} --force-non-host ; \
      fi

RUN  cd ./applications/tari_indexer_web_ui && \
      npm install react-scripts && \
      npm run build

RUN   cd ./applications/tari_validator_node_web_ui && \
      npm install react-scripts && \
      npm run build

RUN      rustup target add wasm32-unknown-unknown && \
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
      --bin tari_validator_node_cli \
      --bin tari_swarm_daemon && \
      # Copy executable out of the cache so it is available in the runtime image.
      cp -v ./target/${BUILD_TARGET}release/tari_indexer /usr/local/bin/ && \
      cp -v ./target/${BUILD_TARGET}release/tari_dan_wallet_daemon /usr/local/bin/ && \
      cp -v ./target/${BUILD_TARGET}release/tari_dan_wallet_cli /usr/local/bin/ && \
      cp -v ./target/${BUILD_TARGET}release/tari_signaling_server /usr/local/bin/ && \
      cp -v ./target/${BUILD_TARGET}release/tari_validator_node /usr/local/bin/ && \
      cp -v ./target/${BUILD_TARGET}release/tari_validator_node_cli /usr/local/bin/ && \
      cp -v ./target/${BUILD_TARGET}release/tari_swarm_daemon /usr/local/bin/ && \
      echo "Tari Dan Build Done"

# Create runtime base minimal image for the target platform executables
FROM --platform=$TARGETPLATFORM rust:$RUST_VERSION-$RUST_DISTRO as runtime

ARG BUILDPLATFORM
ARG TARGETPLATFORM
ARG TARGETOS
ARG TARGETARCH
ARG TARGETVARIANT
ARG RUST_VERSION
ARG RUST_DISTRO

ARG VERSION

# Disable Prompt During Packages Installation
ARG DEBIAN_FRONTEND=noninteractive

# Node Version
ARG NODE_MAJOR
ENV NODE_MAJOR=$NODE_MAJOR

# Prep nodejs 20.x
RUN apt-get update && apt-get install -y \
      apt-transport-https \
      ca-certificates \
      curl \
      gpg && \
      mkdir -p /etc/apt/keyrings && \
      curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg && \
      echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_${NODE_MAJOR}.x nodistro main" | tee /etc/apt/sources.list.d/nodesource.list

RUN apt-get update && apt-get --no-install-recommends install -y \
      libreadline8 \
      libreadline-dev \
      libsqlite3-0 \
      openssl \
      nodejs

RUN rustup target add wasm32-unknown-unknown

RUN rustup toolchain install nightly --force-non-host && \
      rustup target add wasm32-unknown-unknown --toolchain nightly

# Debugging
RUN rustup target list --installed && \
      rustup toolchain list && \
      rustup show

ENV dockerfile_target_platform=$TARGETPLATFORM
ENV dockerfile_version=$VERSION
ENV dockerfile_build_platform=$BUILDPLATFORM
ENV rust_version=$RUST_VERSION

RUN groupadd --gid 1000 tari && \
      useradd --create-home --no-log-init --shell /bin/bash \
      --home-dir /home/tari \
      --uid 1000 --gid 1000 tari

# Setup some folder structure
RUN mkdir -p "/home/tari/sources/tari-connector" && \
      mkdir -p "/home/tari/sources/tari" && \
      mkdir -p "/home/tari/sources/tari-dan" && \
      mkdir -p "/home/tari/data" && \
      chown -R tari:tari "/home/tari/sources" && \
      ln -vsf "/home/tari/sources/tari-connector/" "/usr/lib/node_modules/tari-connector" && \
      mkdir -p "/usr/local/lib/node_modules" && \
      chown -R tari:tari "/usr/local/lib/node_modules"

USER tari
WORKDIR /home/tari

# Debugging
RUN rustup target list --installed && \
      rustup toolchain list && \
      rustup show

# Move into python due to Cross-compile arm64 on amd64 issue
#RUN cargo install cargo-generate

WORKDIR /home/tari/sources
#ADD --chown=tari:tari tari tari
#ADD --chown=tari:tari tari-dan tari-dan
ADD --chown=tari:tari sources/tari-connector /home/tari/sources/tari-connector

WORKDIR /home/tari/sources/tari-connector
RUN npm link

WORKDIR /home/tari/sources/tari-dan
RUN npm link tari-connector

COPY --from=builder-tari /usr/local/bin/minotari_* /usr/local/bin/
COPY --from=builder-tari-dan /usr/local/bin/tari_* /usr/local/bin/

ARG DAN_TESTING_WEBUI_PORT=18000

ENV DAN_TESTING_STEPS_CREATE_ACCOUNT=True
ENV DAN_TESTING_STEPS_RUN_TARI_CONNECTOR_TEST_SITE=True
ENV DAN_TESTING_USE_BINARY_EXECUTABLE=True
ENV DAN_TESTING_NONINTERACTIVE=True
ENV DAN_TESTING_DATA_FOLDER=/home/tari/data
ENV TARI_BINS_FOLDER=/usr/local/bin/
ENV TARI_DAN_BINS_FOLDER=/usr/local/bin/
ENV USER=tari
EXPOSE $DAN_TESTING_WEBUI_PORT
EXPOSE 18001-18025

# TODO: We should put the config in a volume to allow custom configuration but we apply these overrides anyway, so leaving as a TODO
RUN /usr/local/bin/tari_swarm_daemon init \
    --webui-listen-address=0.0.0.0:$DAN_TESTING_WEBUI_PORT \
    --no-compile --binaries-root=/usr/local/bin --start-port=18001
# Set the config overrides on docker start incase they differ from the config
CMD /usr/local/bin/tari_swarm_daemon start \
    --webui-listen-address=0.0.0.0:$DAN_TESTING_WEBUI_PORT \
    --no-compile --binaries-root=/usr/local/bin --start-port=18001