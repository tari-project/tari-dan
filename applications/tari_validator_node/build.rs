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

use std::{env, process::Command};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../tari_validator_node_web_ui/src");
    println!("cargo:rerun-if-changed=../tari_validator_node_web_ui/public");

    if env::var_os("CARGO_FEATURE_TS").is_some() {
        println!("cargo:warning=The web ui is not being compiled when we are generating typescript types/interfaces.");
        return Ok(());
    }

    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };

    if let Err(error) = Command::new(npm)
        .arg("ci")
        .current_dir("../tari_validator_node_web_ui")
        .status()
    {
        println!("cargo:warning='npm ci' error : {:?}", error);
        std::process::exit(1);
    }
    match Command::new(npm)
        .args(["run", "build"])
        .current_dir("../tari_validator_node_web_ui")
        .output()
    {
        Ok(output) if !output.status.success() => {
            println!("cargo:warning='npm run build' exited with non-zero status code");
            println!("cargo:warning=Output: {}", String::from_utf8_lossy(&output.stdout));
            println!("cargo:warning=Error: {}", String::from_utf8_lossy(&output.stderr));
            std::process::exit(1);
        },
        Err(error) => {
            println!("cargo:warning='npm run build' error : {:?}", error);
            println!("cargo:warning=The web ui will not be included!");
            std::process::exit(1);
        },
        _ => {},
    }
    Ok(())
}
