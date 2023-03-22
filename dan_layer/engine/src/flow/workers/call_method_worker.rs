//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, convert::TryFrom, sync::Arc};

use d3ne::{Node, OutputValue, Worker};
use tari_dan_common_types::services::template_provider::TemplateProvider;
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    models::{ComponentAddress, TemplateAddress},
    Hash,
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
            .or(node.get_data("self")?.map(|v| OutputValue::Bytes(v)))
            .ok_or_else(|| anyhow::anyhow!("could not find arg `self`"))?;
        dbg!(&component_address);
        let component_address = ComponentAddress::try_from(component_address.as_bytes()?.to_vec())?;

        let method_name = &node
            .get_data::<String>("method")?
            .ok_or_else(|| anyhow::anyhow!("could not find arg `method`"))?;
        // TODO: There might be a better way to get the template, but for now, you must specify it
        // on the node...
        // let template_address: TemplateAddress = node
        //     .get_data::<String>("template")?
        //     .ok_or_else(|| anyhow::anyhow!("Template not set in data"))?
        //     .parse()?;
        //
        // let function_definition = context
        //     .template_provider
        //     .get_template_module(&template_address)?
        //     .template_def()
        //     .functions
        //     .iter()
        //     .find(|f| f.name == *method_name)
        //     .ok_or_else(|| anyhow::anyhow!("could not find method"))?;
        //
        // let mut args = Vec::new();
        // for arg in function_definition.arguments.iter() {
        //     let arg_value = input_data
        //         .get(arg.name.as_str())
        //         .cloned()
        //         .or(node.get_data(arg.name.as_str())?.map(|v| OutputValue::Bytes(v)))
        //         .ok_or_else(|| anyhow::anyhow!("could not find arg `{}`", arg.name))?;
        //     args.push(arg_value);
        // }

        let exec_result = TransactionProcessor::call_method(
            context.template_provider.clone(),
            &context.runtime,
            context.auth_scope.clone(),
            &component_address,
            &method_name,
            // TODO: put in rest of args
            vec![],
            context.recursion_depth + 1,
            context.max_recursion_depth,
        )?;

        let workspace_key = format!("node[{}].default", node.id);
        // put output on worktop.
        TransactionProcessor::<TTemplateProvider>::put_output_on_workspace_with_name(
            &context.runtime,
            format!("node[{}].default", node.id).into_bytes(),
        )?;

        dbg!(&exec_result);
        let mut h = HashMap::new();
        h.insert(
            "default".to_string(),
            OutputValue::String(format!("workspace::{}", workspace_key)),
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
