//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, sync::Arc};

use d3ne::{InputData, Node, OutputValue, Worker};

use crate::flow::FlowContext;

pub struct CallMethodWorker {}

impl Worker<FlowContext> for CallMethodWorker {
    fn name(&self) -> &str {
        "tari::dan::call_method"
    }

    fn work(
        &self,
        context: &FlowContext,
        node: &Node,
        input_data: HashMap<String, OutputValue>,
    ) -> Result<HashMap<String, OutputValue>, anyhow::Error> {
        todo!()
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
