//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, sync::Arc};

use tari_dan_common_types::services::template_provider::TemplateProvider;

use crate::{function_definitions::FunctionArgDefinition, runtime::Runtime, template::LoadedTemplate};

pub struct FlowContext<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>> {
    pub template_provider: Arc<TTemplateProvider>,
    pub runtime: Runtime,
    pub args: HashMap<String, (tari_bor::Value, FunctionArgDefinition)>,
    pub call_depth: usize,
    pub max_call_depth: usize,
}
