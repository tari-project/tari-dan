//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{env, process::Command};

fn exit_on_ci() {
    if option_env!("CI").is_some() {
        std::process::exit(1);
    }
}

const BUILD: &[(&str, &str)] = &[("./webui", "build")];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=./webui/src");

    if env::var("CARGO_FEATURE_TS").is_ok() {
        println!("cargo:warning=The web ui is not being compiled when we are generating typescript types/interfaces.");
        return Ok(());
    }

    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };

    for (target, build_cmd) in BUILD {
        if let Err(error) = Command::new(npm).arg("ci").current_dir(target).status() {
            println!("cargo:warning='npm ci' error : {:?}", error);
            exit_on_ci();
        }
        match Command::new(npm).args(["run", build_cmd]).current_dir(target).output() {
            Ok(output) if !output.status.success() => {
                println!("cargo:warning='npm run build' exited with non-zero status code");
                println!("cargo:warning=Output: {}", String::from_utf8_lossy(&output.stdout));
                println!("cargo:warning=Error: {}", String::from_utf8_lossy(&output.stderr));
                exit_on_ci();
                break;
            },
            Err(error) => {
                println!("cargo:warning='npm run build' error : {:?}", error);
                println!("cargo:warning=The web ui will not be included!");
                exit_on_ci();
                break;
            },
            _ => {},
        }
    }
    Ok(())
}
