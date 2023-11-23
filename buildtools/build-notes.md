## Build Notes we don't want to lose?

Build options:
 - Native
 - Docker
 - Virtualised
 - Emulated

# Building Linux x86_64 & ARM64

Using Vagrant and VirtualBox has a baseline for building needs, including tools, libs and testing

Linux ARM64 can be built using Vagrant and VirtualBox or Docker and cross

# Prep Ubuntu for development
# From - https://github.com/tari-project/tari-dan/blob/development/scripts/install_ubuntu_dependencies.sh
```bash
sudo apt-get update
sudo DEBIAN_FRONTEND=noninteractive apt-get install --no-install-recommends --assume-yes \
  apt-transport-https \
  ca-certificates \
  curl \
  gpg \
  openssl \
  libssl-dev \
  pkg-config \
  libsqlite3-dev \
  git \
  cmake \
  dh-autoreconf \
  libc++-dev \
  libc++abi-dev \
  libprotobuf-dev \
  protobuf-compiler \
  libncurses5-dev \
  libncursesw5-dev \
  build-essential \
  zip
```

# Install rust
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
# or unattended rust install
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```

```bash
source "$HOME/.cargo/env"
```

# Install wasm prerequisite
```bash
rustup target add wasm32-unknown-unknown
```

# Install nodejs prerequisite
```bash
export NODE_MAJOR=20
sudo mkdir -p /etc/apt/keyrings
curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | \
  sudo gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg
echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_${NODE_MAJOR}.x nodistro main" | \
  sudo tee /etc/apt/sources.list.d/nodesource.list

sudo apt-get update 

sudo DEBIAN_FRONTEND=noninteractive apt-get install --no-install-recommends --assume-yes \
  nodejs
```

# Prep Ubuntu for cross-compile aarch64/arm64 on x86_64
```bash
sudo DEBIAN_FRONTEND=noninteractive apt-get install --no-install-recommends --assume-yes \
  pkg-config-aarch64-linux-gnu \
  gcc-aarch64-linux-gnu \
  g++-aarch64-linux-gnu
```

# Prep rust for cross-compile aarch64/arm64 on x86_64
```bash
rustup target add aarch64-unknown-linux-gnu
rustup toolchain install stable-aarch64-unknown-linux-gnu
```

# Check was tools chains rust has in place
```bash
rustup target list --installed
rustup toolchain list
```

# get/git the code base
```bash
mkdir -p ~/src
cd ~/src
git clone git@github.com:tari-project/tari-dan.git
cd tari-dan
```

# Build Testing
```bash
cargo build \
  --target aarch64-unknown-linux-gnu \
  --bin tari_dan_wallet_cli
```

# Build target Release
```bash
cargo build --locked --release \
  --target aarch64-unknown-linux-gnu
```

# Using a single command line build using Docker
```bash
cross build --locked --release \
  --target aarch64-unknown-linux-gnu
```
