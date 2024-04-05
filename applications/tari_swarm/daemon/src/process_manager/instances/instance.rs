//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::config::InstanceType;

pub type InstanceId = u32;

#[allow(dead_code)]
pub struct Instance {
    id: InstanceId,
    instance_type: InstanceType,
}
