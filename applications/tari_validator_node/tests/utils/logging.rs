//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fs, path::PathBuf, process};

pub fn create_log_config_file() -> PathBuf {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let template = crate_root.join("tests/log4rs/cucumber.yml");
    let contents = fs::read_to_string(template).unwrap();
    let dest_path = crate_root.join(format!("tests/temp/cucumber_{}", process::id()));
    fs::create_dir_all(&dest_path).unwrap();
    let contents = contents.replace(
        "{{log_dir}}",
        dest_path.clone().into_os_string().into_string().unwrap().as_str(),
    );
    let log_config = dest_path.join("log4rs.yml");
    fs::write(&log_config, contents).unwrap();
    log_config
}
