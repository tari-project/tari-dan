// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use d3ne::{Node, OutputValue, Worker};
use tari_bor::from_value;
use tari_dan_common_types::services::template_provider::TemplateProvider;

use crate::{flow::FlowContext, function_definitions::ArgType, template::LoadedTemplate};

pub struct ArgWorker {}

impl<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>> Worker<FlowContext<TTemplateProvider>>
    for ArgWorker
{
    fn work(
        &self,
        context: &FlowContext<TTemplateProvider>,
        node: &Node,
        _input_data: HashMap<String, OutputValue>,
    ) -> Result<HashMap<String, OutputValue>, anyhow::Error> {
        let arg_name: String = node
            .get_data("name")?
            .ok_or_else(|| anyhow::anyhow!("could not find arg `name`"))?;
        let (value, arg_def) = context
            .args
            .get(arg_name.as_str())
            .ok_or_else(|| anyhow::anyhow!("could not find arg"))?;

        let mut result = HashMap::new();
        match arg_def.arg_type {
            ArgType::String => {
                result.insert("default".to_string(), OutputValue::String(from_value(value)?));
            },
            ArgType::Bytes => {
                result.insert("default".to_string(), OutputValue::Bytes(from_value(value)?));
            },
        };
        Ok(result)
    }

    fn name(&self) -> &str {
        "tari::arg"
    }
}
