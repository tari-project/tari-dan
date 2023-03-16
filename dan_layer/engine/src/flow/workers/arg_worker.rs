// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use d3ne::{InputData, Node, OutputData, OutputDataBuilder, OutputValue, Worker};
use tari_template_lib::args::Arg;
use tari_utilities::hex::Hex;

use crate::{
    flow::{ArgValue, FlowContext},
    function_definitions::ArgType,
};

pub struct ArgWorker {}

impl Worker<FlowContext> for ArgWorker {
    // fn call(&self, node: Node, inputs: InputData) -> OutputData {
    //     let name = node.get_string_field("name", &inputs).unwrap();
    //     let mut map = HashMap::new();
    //     let value = self.args.get(&name).cloned().expect("could not find arg");
    //     match value {
    //         ArgValue::Uint(x) => map.insert(
    //             "default".to_string(),
    //             Ok(IOData {
    //                 data: Box::new(x as i64),
    //             }),
    //         ),
    //         ArgValue::PublicKey(pk) => map.insert(
    //             "default".to_string(),
    //             Ok(IOData {
    //                 data: Box::new(pk.to_hex()),
    //             }),
    //         ),
    //         _ => todo!(),
    //     };
    //
    //     Rc::new(map)
    // }

    fn work(
        &self,
        context: &FlowContext,
        node: &Node,
        input_data: HashMap<String, OutputValue>,
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
                result.insert(
                    "default".to_string(),
                    OutputValue::String(String::from_utf8(value.to_bytes())?),
                );
            },
        };
        Ok(result)
    }

    fn name(&self) -> &str {
        "tari::arg"
    }

    // fn work(&self, node: &Node, input_data: InputData) -> anyhow::Result<OutputData> {
    //     let name = node.get_string_field("name", &input_data)?;
    //     let value = self.args.get(&name).cloned().expect("could not find arg");
    //     let output = match value {
    //         ArgValue::Uint(x) => OutputDataBuilder::new().data("default", Box::new(x as i64)),
    //         ArgValue::PublicKey(pk) => OutputDataBuilder::new().data("default", Box::new(pk.to_hex())),
    //         _ => todo!(),
    //     };
    //     Ok(output.build())
    // }
}
