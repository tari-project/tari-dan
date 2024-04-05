//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::process;

pub fn log_error_output(output: &process::Output) {
    log_output(log::Level::Error, output);
    // eprintln!("STDOUT:");
    // let stdout = String::from_utf8_lossy(&output.stdout);
    // eprintln!("{}", stdout);
    // eprintln!("STDERR:");
    // let stderr = String::from_utf8_lossy(&output.stderr);
    // eprintln!("{}", stderr);
    // eprintln!("STATUS: {}", output.status);
}

pub fn log_output(level: log::Level, output: &process::Output) {
    log::log!(level, "STDOUT:");
    let stdout = String::from_utf8_lossy(&output.stdout);
    log::log!(level, "{}", stdout);
    log::log!(level, "STDERR:");
    let stderr = String::from_utf8_lossy(&output.stderr);
    log::log!(level, "{}", stderr);
    log::log!(level, "STATUS: {}", output.status);
}
