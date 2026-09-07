//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Compiles the compute-bound benchmark template to WASM and stages it for `include_bytes!`.
//!
//! Doing this at build time rather than at run time is what keeps `tari-vn-bench` a single
//! transferable file: the machine under test needs no Rust toolchain, no checkout and no network.
//! The build host needs the `wasm32-unknown-unknown` target, which working in this repository
//! already requires.

use std::{
    env,
    fs,
    io,
    path::{Path, PathBuf},
    process::Command,
};

const TEMPLATE: &str = "templates/compute_bench";

fn main() -> io::Result<()> {
    let crate_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is always set"));
    let template_path = crate_path.join(TEMPLATE);

    println!("cargo:rerun-if-changed={TEMPLATE}/src");
    println!("cargo:rerun-if-changed={TEMPLATE}/Cargo.toml");
    // The template links against the workspace's template lib, so a change there changes the
    // compiled artifact even when the template's own sources have not moved.
    println!("cargo:rerun-if-changed=../../crates/template_lib");

    compile(&template_path)?;

    let compiled_dir = crate_path.join("compiled");
    fs::create_dir_all(&compiled_dir)?;
    let built = template_path
        .join("target/wasm32-unknown-unknown/release")
        .join("compute_bench.wasm");
    fs::copy(built, compiled_dir.join("compute_bench.wasm"))?;

    Ok(())
}

fn compile(package_dir: &Path) -> io::Result<()> {
    // The template compiles to wasm32, whose linker is rust-lld. Cargo hands this build script the
    // host rustflags in CARGO_ENCODED_RUSTFLAGS, and that variable applies to every target, so
    // anything host-specific in it (a `-fuse-ld`, a `-C target-cpu`) would reach the wasm link and
    // be rejected.
    let output = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
        .current_dir(package_dir)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
        .output()?;

    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        return Err(io::Error::other(format!(
            "failed to compile the benchmark template at {}; is the wasm32-unknown-unknown target installed?",
            package_dir.display(),
        )));
    }
    Ok(())
}
