//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

fn main() {
    println!("cargo:rustc-env=TARI_NETWORK=localnet");
    println!("cargo:rustc-env=TARI_TARGET_NETWORK=localnet");
}
