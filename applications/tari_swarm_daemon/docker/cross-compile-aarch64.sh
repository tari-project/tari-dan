#!/usr/bin/env bash
#
# Move all cross-compiling steps into a single script
# Hardcoded ARM64 envs for cross-compiling on x86_64
#

set -e

export BUILD_TARGET="aarch64-unknown-linux-gnu/"
export RUST_TARGET="--target=aarch64-unknown-linux-gnu"
#export ARCH=${ARCH:-generic}
export ARCH=generic
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
export BINDGEN_EXTRA_CLANG_ARGS="--sysroot /usr/aarch64-linux-gnu/include/"
export RUSTFLAGS="-C target_cpu=$ARCH"
export ROARING_ARCH=$ARCH
rustup target add aarch64-unknown-linux-gnu
rustup toolchain install stable-aarch64-unknown-linux-gnu --force-non-host

# Check for Debian
if [ -f "/etc/debian_version" ] ; then
  dpkg --add-architecture arm64
  apt-get update
  apt-get install -y pkg-config libssl-dev:arm64
  export AARCH64_UNKNOWN_LINUX_GNU_OPENSSL_INCLUDE_DIR=/usr/include/aarch64-linux-gnu/openssl/
  export PKG_CONFIG_ALLOW_CROSS=1
fi
