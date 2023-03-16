// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    ops::Deref,
    sync::{Arc, RwLock},
};

use d3ne::{Engine, Node, Workers, WorkersBuilder};
use serde_json::Value as JsValue;
use tari_common_types::types::PublicKey;
use tari_engine_types::execution_result::ExecutionResult;
use tari_template_lib::args::Arg;
use tari_utilities::ByteArray;

use crate::{
    flow,
    flow::{
        workers::{ArgWorker, CallMethodWorker},
        ArgValue,
        FlowContext,
        FlowEngineError,
    },
    function_definitions::FunctionArgDefinition,
    packager::Package,
    runtime::{AuthorizationScope, Runtime},
};

#[derive(Clone, Debug)]
pub struct FlowInstance {
    // engine: Engine,
    // TODO: engine is not Send so can't be added here
    // process: JsValue,
    start_node: i64,
    nodes: HashMap<i64, Node>,
}

impl FlowInstance {
    pub fn try_build(value: JsValue, workers: Workers<FlowContext>) -> Result<Self, FlowEngineError> {
        let engine = Engine::new("tari_engine@0.1.0".to_string(), workers);
        // dbg!(&value);
        let nodes = engine.parse_value(value).expect("could not create engine");
        Ok(FlowInstance {
            // process: value,
            nodes,
            start_node: 1,
        })
    }

    pub fn invoke(
        &self,
        package: Package,
        runtime: Runtime,
        auth_scope: AuthorizationScope,
        args: &[Arg],
        arg_defs: &[FunctionArgDefinition],
        recursion_depth: usize,
        max_recursion_depth: usize,
    ) -> Result<ExecutionResult, FlowEngineError> {
        // let mut engine_args = HashMap::new();

        // let mut remaining_args = Vec::from(args);
        // for ad in arg_defs {
        //     let value = match ad.arg_type {
        //         ArgType::String => {
        //             let length = remaining_args.pop().expect("no more args: len") as usize;
        //             let s_bytes: Vec<u8> = remaining_args.drain(0..length).collect();
        //             let s = String::from_utf8(s_bytes).expect("could not convert string");
        //             ArgValue::String(s)
        //         },
        //             ArgType::Byte => ArgValue::Byte(remaining_args.pop().expect("No byte to read")),
        //             ArgType::PublicKey => {
        //                 let bytes: Vec<u8> = remaining_args.drain(0..32).collect();
        //                 let pk = PublicKey::from_bytes(&bytes).expect("Not a valid public key");
        //                 ArgValue::PublicKey(pk)
        //             },
        //             ArgType::Uint => {
        //                 let bytes: Vec<u8> = remaining_args.drain(0..8).collect();
        //                 let mut fixed: [u8; 8] = [0u8; 8];
        //                 fixed.copy_from_slice(&bytes);
        //                 let value = u64::from_le_bytes(fixed);
        //                 ArgValue::Uint(value)
        //             },
        //         };
        //         engine_args.insert(ad.name.clone(), value);
        //     }
        //
        // let state_db = Arc::new(RwLock::new(state_db));
        let engine = Engine::new("tari@0.1.0".to_string(), load_workers());
        let mut args_map = HashMap::new();
        for (i, arg_def) in arg_defs.iter().enumerate() {
            if i >= args.len() {
                return Err(FlowEngineError::MissingArgument {
                    name: arg_def.name.clone(),
                });
            }
            args_map.insert(arg_def.name.clone(), (args[i].clone(), arg_def.clone()));
        }
        let context = FlowContext {
            package,
            runtime,
            auth_scope,
            args: args_map,
            recursion_depth,
            max_recursion_depth,
        };
        engine.process(&context, &self.nodes, self.start_node).unwrap();
        //     let output = engine.process(&self.nodes, self.start_node);
        //     let _od = output.expect("engine process failed");
        //     if let Some(err) = od.get("error") {
        //     match err {
        //         Ok(_) => todo!("Unexpected Ok result returned from error"),
        //         Err(e) => {
        //             return Err(FlowEngineError::InstructionFailed { inner: e.to_string() });
        //         },
        //     }
        // }
        // let inner = state_db.read().map(|s| s.deref().clone()).unwrap();
        // Ok(inner)
        todo!()
    }
}

fn load_workers() -> Workers<FlowContext> {
    let mut workers = WorkersBuilder::default();

    // workers.add(StartWorker {});
    workers.add(CallMethodWorker {});
    // workers.add(CreateBucketWorker {
    //     state_db: state_db.clone(),
    // });
    // workers.add(StoreBucketWorker {
    //     state_db: state_db.clone(),
    // });
    // workers.add(ArgWorker { args: args.clone() });
    workers.add(ArgWorker {});
    // workers.add(SenderWorker { sender });
    // workers.add(TextWorker {});
    // workers.add(HasRoleWorker { state_db });
    // workers.add(MintBucketWorker {});
    workers.build()
}
