// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{env, fs, process::Command};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=./web_ui/src");
    println!("cargo:rerun-if-changed=./web_ui/public");
    println!("cargo:rerun-if-changed=./web_ui/scripts");
    // The UI embeds a decoder for the engine's types, so it is rebuilt with the crates that define them.
    println!("cargo:rerun-if-changed=../../crates/ootle_wasm");
    println!("cargo:rerun-if-changed=../../crates/engine_types/src");
    println!("cargo:rerun-if-changed=../../crates/template_lib_types/src");

    fs::create_dir_all("./web_ui/dist")?;

    if env::var("CARGO_FEATURE_TS").is_ok() {
        println!("cargo:warning=The web ui will not be built because the tx feature is enabled.");
        return Ok(());
    }
    if env::var("CARGO_FEATURE_WEB_UI").is_err() {
        println!("cargo:warning=The web ui will not be built because the web_ui feature is not enabled.");
        return Ok(());
    }

    if cfg!(debug_assertions) {
        println!("cargo:warning=The web ui will not be compiled in debug mode.");
        return Ok(());
    }
    #[cfg(windows)]
    const NPM: &str = "pnpm.cmd";
    #[cfg(not(windows))]
    const NPM: &str = "pnpm";

    if let Err(error) = Command::new(NPM).arg("install").current_dir("./web_ui").status() {
        println!("cargo:warning='npm install' error : {:?}", error);
        return Ok(());
    }
    match Command::new(NPM)
        .args(["run", "build"])
        .current_dir("./web_ui")
        .output()
    {
        Ok(output) if !output.status.success() => {
            println!("cargo:warning='npm run build' exited with non-zero status code");
            println!("cargo:warning=Output: {}", String::from_utf8_lossy(&output.stdout));
            println!("cargo:warning=Error: {}", String::from_utf8_lossy(&output.stderr));
        },
        Err(error) => {
            println!(
                "cargo:warning='{NPM} run build' error (is {NPM} installed?): {:?}",
                error
            );
            println!("cargo:warning=The web ui will not be included!");
        },
        _ => {},
    }
    Ok(())
}
