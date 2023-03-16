//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use tari_template_lib::args::Arg;

use crate::{
    function_definitions::FunctionArgDefinition,
    packager::Package,
    runtime::{AuthorizationScope, Runtime},
};

pub struct FlowContext {
    pub package: Package,
    pub runtime: Runtime,
    pub auth_scope: AuthorizationScope,
    pub args: HashMap<String, (Arg, FunctionArgDefinition)>,
    pub recursion_depth: usize,
    pub max_recursion_depth: usize,
}
