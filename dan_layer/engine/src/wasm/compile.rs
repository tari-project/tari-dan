//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    fs,
    fs::File,
    io,
    io::{ErrorKind, Write},
    path::Path,
    process::Command,
};

use cargo_toml::{Manifest, Product};
use tempfile::tempdir;

use super::module::WasmModule;

// TODO: remove from main build
pub fn compile_str<S: AsRef<str>>(source: S, features: &[&str]) -> Result<WasmModule, io::Error> {
    let source = source.as_ref();
    let temp_dir = tempdir()?;

    fs::create_dir_all(temp_dir.path().join("src"))?;
    File::create(temp_dir.path().join("src/lib.rs"))?.write_all(source.as_bytes())?;
    // super hacky
    File::create(temp_dir.path().join("Cargo.toml"))?.write_all(
            br#"
        [workspace]
[package]
name = "temp_crate_lib"
version = "0.1.0"
edition = "2021"

[dependencies]
tari_template_abi = { git="https://github.com/tari-project/tari-dan.git", package="tari_template_abi", default-features = false, rev="9ef9cccfed5390f61b1a28aa3e04cde6813016ef" }
tari_template_lib = { git="https://github.com/tari-project/tari-dan.git", package="tari_template_lib", rev = "9ef9cccfed5390f61b1a28aa3e04cde6813016ef" }
tari_template_macros = { git="https://github.com/tari-project/tari-dan.git", package="tari_template_macros", rev = "9ef9cccfed5390f61b1a28aa3e04cde6813016ef" }

[profile.release]
opt-level = 's'     # Optimize for size.
lto = true          # Enable Link Time Optimization.
codegen-units = 1   # Reduce number of codegen units to increase optimizations.
panic = 'abort'     # Abort on panic.
strip = "debuginfo" # Strip debug info.

[lib]
crate-type = ["cdylib", "lib"]
        "#
    )?;

    compile_template(temp_dir.path(), features)
}

pub fn compile_template<P: AsRef<Path>>(package_dir: P, features: &[&str]) -> io::Result<WasmModule> {
    let mut args = ["build", "--target", "wasm32-unknown-unknown", "--release"]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    if !features.is_empty() {
        args.push("--features".to_string());
        args.extend(features.iter().map(ToString::to_string));
    }

    let output = Command::new("cargo")
        .current_dir(package_dir.as_ref())
        .args(args)
        .output()?;
    if !output.status.success() {
        eprintln!("stdout:");
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr:");
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        return Err(io::Error::new(
            ErrorKind::Other,
            format!("Failed to compile package: {:?}", package_dir.as_ref(),),
        ));
    }

    // resolve wasm name
    let manifest = Manifest::from_path(package_dir.as_ref().join("Cargo.toml")).unwrap();
    let wasm_name = if let Some(Product { name: Some(name), .. }) = manifest.lib {
        // lib name
        name
    } else if let Some(pkg) = manifest.package {
        // package name
        pkg.name.replace('-', "_")
    } else {
        // file name
        package_dir
            .as_ref()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
            .replace('-', "_")
    };

    // path of the wasm executable
    let mut path = package_dir.as_ref().to_path_buf();
    path.push("target");
    path.push("wasm32-unknown-unknown");
    path.push("release");
    path.push(wasm_name);
    path.set_extension("wasm");

    // return
    let code = fs::read(path)?;
    Ok(WasmModule::from_code(code))
}
