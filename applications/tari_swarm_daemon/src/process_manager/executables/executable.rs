//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use crate::config::InstanceType;

pub struct Executable {
    pub instance_type: InstanceType,
    pub path: PathBuf,
    pub env: Vec<(String, String)>,
}
