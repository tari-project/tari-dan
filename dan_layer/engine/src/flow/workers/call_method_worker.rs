//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, convert::TryFrom};

use d3ne::{Node, OutputValue, Worker};
use tari_dan_common_types::services::template_provider::TemplateProvider;
use tari_template_lib::{
    args::Arg,
    models::{ComponentAddress, TemplateAddress},
};

use crate::{flow::FlowContext, packager::LoadedTemplate, transaction::TransactionProcessor};

pub struct CallMethodWorker {}

impl<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>> Worker<FlowContext<TTemplateProvider>>
    for CallMethodWorker
{
    fn name(&self) -> &str {
        "tari::dan::call_method"
    }

    fn work(
        &self,
        context: &FlowContext<TTemplateProvider>,
        node: &Node,
        input_data: HashMap<String, OutputValue>,
    ) -> Result<HashMap<String, OutputValue>, anyhow::Error> {
        let component_address = input_data
            .get("self")
            .cloned()
            .or(node.get_data("self")?.map(OutputValue::Bytes))
            .ok_or_else(|| anyhow::anyhow!("could not find arg `self`"))?;
        dbg!(&component_address);
        let component_address = ComponentAddress::try_from(component_address.as_bytes()?.to_vec())?;

        let method_name = &node
            .get_data::<String>("method")?
            .ok_or_else(|| anyhow::anyhow!("could not find arg `method`"))?;
        // TODO: There might be a better way to get the template, but for now, you must specify it
        // on the node...
        let mut template_hash = node
            .get_data::<String>("template")?
            .ok_or_else(|| anyhow::anyhow!("Template not set in data"))?;
        if template_hash.starts_with("0x") {
            template_hash = template_hash[2..].to_string();
        }
        let template_address: TemplateAddress = TemplateAddress::from_hex(&template_hash).map_err(|e| {
            anyhow::anyhow!(format!(
                "Template address `{}` was not a valid hash:{}",
                &template_hash, e
            ))
        })?;

        let function_definition = context
            .template_provider
            .get_template_module(&template_address)?
            .template_def()
            .functions
            .iter()
            .find(|f| f.name == *method_name)
            .ok_or_else(|| anyhow::anyhow!("could not find method"))?
            .clone();

        let mut args = Vec::new();
        for arg in function_definition.arguments.iter() {
            if arg.name == "self" {
                // self has already been added
                continue;
            }
            let arg_value = input_data
                .get(arg.name.as_str())
                .cloned()
                .or(node.get_data(arg.name.as_str())?.map(OutputValue::Bytes))
                .ok_or_else(|| anyhow::anyhow!("could not find arg `{}`", arg.name))?;
            args.push(Arg::Literal(arg_value.as_bytes()?.to_vec()));
        }

        dbg!(&function_definition);
        dbg!(&args);

        let exec_result = TransactionProcessor::call_method(
            context.template_provider.clone(),
            &context.runtime,
            context.auth_scope.clone(),
            &component_address,
            method_name,
            // TODO: put in rest of args
            args,
            context.recursion_depth + 1,
            context.max_recursion_depth,
        )?;

        let result = exec_result.raw.clone();
        // let workspace_key = format!("node[{}].default", node.id);
        // // put output on worktop.
        // TransactionProcessor::<TTemplateProvider>::put_output_on_workspace_with_name(
        //     &context.runtime,
        //     format!("node[{}].default", node.id).into_bytes(),
        // )?;

        dbg!(&exec_result);
        let mut h = HashMap::new();
        h.insert(
            "default".to_string(),
            // OutputValue::String(format!("workspace::{}", workspace_key)),
            OutputValue::Bytes(result),
        );
        Ok(h)
    }

    // fn work(&self, context: &FlowContext, node: &Node, input_data: InputData) -> anyhow::Result<OutputData> {
    //     let component_address = node.get_string_field("component_address", &input_data)?;
    //         .component_address
    //         .clone()
    //         .unwrap_or_else(|| node.get_string_field("component_address", &input_data)?);
    //     todo!()
    // let name = node.get_string_field("name", &input_data)?;
    // let value = self.args.get(&name).cloned().expect("could not find arg");
    // let output = match value {
    //     ArgValue::Uint(x) => OutputDataBuilder::new().data("default", Box::new(x as i64)),
    //     ArgValue::PublicKey(pk) => OutputDataBuilder::new().data("default", Box::new(pk.to_hex())),
    //     _ => todo!(),
    // };
    // Ok(output.build())
    // }
}
