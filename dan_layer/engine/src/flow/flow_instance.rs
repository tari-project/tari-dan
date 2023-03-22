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
use tari_dan_common_types::services::template_provider::TemplateProvider;
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
    packager::{LoadedTemplate, Package},
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
    pub fn try_build<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>(
        value: JsValue,
        workers: Workers<FlowContext<TTemplateProvider>>,
    ) -> Result<Self, FlowEngineError> {
        let engine = Engine::new("tari_engine@0.1.0".to_string(), workers);
        // dbg!(&value);
        let nodes = engine.parse_value(value).expect("could not create engine");
        Ok(FlowInstance {
            // process: value,
            nodes,
            start_node: 1,
        })
    }

    pub fn invoke<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>(
        &self,
        template_provider: Arc<TTemplateProvider>,
        runtime: Runtime,
        auth_scope: AuthorizationScope,
        args: &[Arg],
        arg_defs: &[FunctionArgDefinition],
        recursion_depth: usize,
        max_recursion_depth: usize,
    ) -> Result<ExecutionResult, FlowEngineError> {
        let engine = Engine::new("tari@0.1.0".to_string(), load_workers());
        let args = runtime.resolve_args(args.to_vec())?;
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
            template_provider,
            runtime,
            auth_scope,
            args: args_map,
            recursion_depth,
            max_recursion_depth,
        };
        let result = engine.process(&context, &self.nodes, self.start_node)?;

        // TODO: return actual result from flow
        Ok(ExecutionResult::empty())
    }
}

fn load_workers<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>(
) -> Workers<FlowContext<TTemplateProvider>> {
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
