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
    env,
    error::Error,
    fs,
    io,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

const TEMPLATE_BUILTINS: &[&str] = &["templates/account"];

fn main() -> Result<(), Box<dyn Error>> {
    for template in TEMPLATE_BUILTINS {
        // we only want to rebuild if a template was added/modified
        println!("cargo:rerun-if-changed={}/src", template);

        let template_path = env::current_dir()?.join(template);

        // compile the template into wasm
        compile_template(&template_path)?;

        // get the path of the wasm executable
        let wasm_name = Path::new(template).file_name().unwrap().to_str().unwrap();
        let wasm_path = get_compiled_wasm_path(&template_path, wasm_name);

        // copy the wasm binary to the root folder of the template, to be included in source control
        let wasm_dest = template_path.join(wasm_name).with_extension("wasm");
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
