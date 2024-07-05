//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    env,
    error::Error,
    fs,
    io,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

const TEMPLATE_BUILTINS: &[&str] = &["templates/account", "templates/account_nfts"];

fn main() -> Result<(), Box<dyn Error>> {
    // Rebuild templates if abi or lib changes
    println!("cargo:rerun-if-changed=../template_abi");
    println!("cargo:rerun-if-changed=../template_lib");
    for template in TEMPLATE_BUILTINS {
        // we only want to rebuild if a template was added/modified
        println!("cargo:rerun-if-changed={}/src", template);
        println!("cargo:rerun-if-changed={}/Cargo.toml", template);

        let template_path = env::current_dir()?.join(template);

        // compile the template into wasm
        compile_template(&template_path)?;

        // get the path of the wasm executable
        let wasm_name = Path::new(template).file_name().unwrap().to_str().unwrap();
        let wasm_path = get_compiled_wasm_path(&template_path, wasm_name);

        // copy the wasm binary to the root folder of the template, to be included in source control
        let wasm_dest = template_path.join(wasm_name).with_extension("wasm");
        if wasm_dest.exists() {
            let existing_contents = fs::read(&wasm_dest)?;
            let dest_contents = fs::read(&wasm_path)?;
            if existing_contents == dest_contents {
                continue;
            }
        }
        fs::copy(wasm_path, wasm_dest)?;
    }

    Ok(())
}

fn compile_template<P: AsRef<Path>>(package_dir: P) -> Result<(), Box<dyn Error>> {
    let args = ["build", "--target", "wasm32-unknown-unknown", "--release"];

    let output = Command::new("cargo")
        .current_dir(package_dir.as_ref())
        .args(args)
        .output()?;

    if !output.status.success() {
        eprintln!("stdout:");
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr:");
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        return Err(Box::new(io::Error::new(
            ErrorKind::Other,
            format!("Failed to compile package: {:?}", package_dir.as_ref(),),
        )));
    }

    Ok(())
}

fn get_compiled_wasm_path<P: AsRef<Path>>(template_path: P, wasm_name: &str) -> PathBuf {
    template_path
        .as_ref()
        .join("target")
        .join("wasm32-unknown-unknown")
        .join("release")
        .join(wasm_name)
        .with_extension("wasm")
}
