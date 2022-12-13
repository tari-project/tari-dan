//  Copyright 2022, The Tari Project
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
    error::Error,
    fs::{self},
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    process::Command,
};

const TEMPLATES_FOLDER_NAME: &str = "templates";

fn main() -> Result<(), Box<dyn Error>> {
    // we only want to rebuild if a template was added/modified/deleted
    println!("cargo:rerun-if-changed={}", TEMPLATES_FOLDER_NAME);

    let templates_folder = std::env::current_dir().unwrap().join(TEMPLATES_FOLDER_NAME);
    let templates_folder_dir = fs::read_dir(templates_folder).unwrap();
    for template_path in templates_folder_dir {
        let path_buf = template_path?.path();
        let package_dir = path_buf.as_path();
        let wasm_name = package_dir.file_name().unwrap().to_str().unwrap();

        // compile the template into wasm
        compile_template(&package_dir)?;

        // get the path of the wasm executable
        let wasm_path = get_compiled_wasm_path(&package_dir, wasm_name)?;

        // copy the wasm binary to the root folder of the template, to be included in source control
        let mut wasm_dest = package_dir.join(wasm_name);
        wasm_dest.set_extension("wasm");
        fs::copy(wasm_path, wasm_dest)?;
    }

    Ok(())
}

fn compile_template<P: AsRef<Path>>(package_dir: &P) -> Result<(), Box<dyn Error>> {
    let args = ["build", "--target", "wasm32-unknown-unknown", "--release"]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

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

fn get_compiled_wasm_path<P: AsRef<Path>>(template_path: &P, wasm_name: &str) -> Result<PathBuf, Box<dyn Error>> {
    let mut wasm_path = template_path.as_ref().to_path_buf();
    wasm_path.push("target");
    wasm_path.push("wasm32-unknown-unknown");
    wasm_path.push("release");
    wasm_path.push(wasm_name);
    wasm_path.set_extension("wasm");

    Ok(wasm_path)
}
