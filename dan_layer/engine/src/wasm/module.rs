//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{fmt, fmt::Formatter, sync::Arc};

use tari_template_abi::{FunctionDef, TemplateDef, ABI_TEMPLATE_DEF_GLOBAL_NAME};
use wasmer::{
    imports,
    sys::BaseTunables,
    AsStoreMut,
    CompilerConfig,
    Cranelift,
    CraneliftOptLevel,
    Engine,
    ExportError,
    Function,
    Instance,
    NativeEngineExt,
    Pages,
    Store,
    Target,
    TypedFunction,
    WasmPtr,
};

use crate::{
    template::{LoadedTemplate, TemplateLoaderError, TemplateModuleLoader},
    wasm::{environment::WasmEnv, limiting_tunable::LimitingTunables, metering, WasmExecutionError},
};

pub type MainFunction = TypedFunction<(WasmPtr<u8>, u32), WasmPtr<u8>>;
#[derive(Debug, Clone)]
pub struct WasmModule {
    code: Vec<u8>,
}

impl WasmModule {
    pub fn from_code(code: Vec<u8>) -> Self {
        Self { code }
    }

    pub fn load_template_from_code(code: &[u8]) -> Result<LoadedTemplate, TemplateLoaderError> {
        let engine = Self::create_engine();
        let module = wasmer::Module::new(&engine, code)?;
        let mut store = Store::new(engine);

        let imports = imports! {
            "env" => {
                "tari_engine" => Function::new_typed(&mut store, |_op: i32, _arg_ptr: i32, _arg_len: i32| 0i32),
                "debug" => Function::new_typed(&mut store, |_arg_ptr: i32, _arg_len: i32| {  }),
                "on_panic" => Function::new_typed(&mut store, |_msg_ptr: i32, _msg_len: i32, _line: i32, _col: i32| {  }),
            }
        };
        let instance = Instance::new(&mut store, &module, &imports)?;
        let mut env = WasmEnv::new(());
        let memory = instance.exports.get_memory("memory")?.clone();
        env.set_memory(memory);
        let template = env.load_abi(&mut store, &instance)?;
        let main_fn = format!("{}_main", template.template_name());
        validate_instance(&mut store, &instance, &main_fn)?;

        let engine = store.engine().clone();

        Ok(LoadedWasmTemplate::new(template, module, engine, code.len()).into())
    }

    pub fn code(&self) -> &[u8] {
        &self.code
    }

    fn create_engine() -> Engine {
        const MEMORY_PAGE_LIMIT: Pages = Pages(32); // 2MiB = 32 * 65,536
        let base = BaseTunables::for_target(&Target::default());
        let tunables = LimitingTunables::new(base, MEMORY_PAGE_LIMIT);
        let mut compiler = Cranelift::new();
        compiler.opt_level(CraneliftOptLevel::Speed).canonicalize_nans(true);
        // TODO: Configure metering limit
        compiler.push_middleware(Arc::new(metering::middleware(100_000_000)));
        let mut engine = Engine::from(compiler);
        engine.set_tunables(tunables);

        engine
    }
}

impl TemplateModuleLoader for WasmModule {
    fn load_template(&self) -> Result<LoadedTemplate, TemplateLoaderError> {
        Self::load_template_from_code(&self.code)
    }
}

#[derive(Clone)]
pub struct LoadedWasmTemplate {
    template_def: Arc<TemplateDef>,
    module: wasmer::Module,
    engine: Engine,
    code_size: usize,
}

impl LoadedWasmTemplate {
    pub fn new(template_def: TemplateDef, module: wasmer::Module, engine: Engine, code_size: usize) -> Self {
        Self {
            template_def: Arc::new(template_def),
            module,
            engine,
            code_size,
        }
    }

    pub fn wasm_module(&self) -> &wasmer::Module {
        &self.module
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn create_store(&self) -> Store {
        Store::new(self.engine.clone())
    }

    pub fn template_name(&self) -> &str {
        self.template_def.template_name()
    }

    pub fn template_def(&self) -> &TemplateDef {
        &self.template_def
    }

    pub fn find_func_by_name(&self, function_name: &str) -> Option<&FunctionDef> {
        self.template_def.functions().iter().find(|f| f.name == *function_name)
    }

    pub fn code_size(&self) -> usize {
        self.code_size
    }
}

impl fmt::Debug for LoadedWasmTemplate {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedWasmTemplate")
            .field("template_name", &self.template_name())
            .field("code_size", &self.code_size())
            .field("main", &"<main func>")
            .field("module", &self.module)
            .finish()
    }
}

fn validate_instance<S: AsStoreMut>(
    store: &mut S,
    instance: &Instance,
    main_fn: &str,
) -> Result<(), WasmExecutionError> {
    fn is_func_permitted(name: &str) -> bool {
        name.ends_with("_main") || name == "tari_alloc" || name == "tari_free"
    }

    // Enforce that only permitted functions are allowed
    let unexpected_abi_func = instance
        .exports
        .iter()
        .functions()
        .find(|(name, _)| !is_func_permitted(name));

    if let Some((name, _)) = unexpected_abi_func {
        return Err(WasmExecutionError::UnexpectedAbiFunction { name: name.to_string() });
    }

    instance
        .exports
        .get_global(ABI_TEMPLATE_DEF_GLOBAL_NAME)?
        .get(store)
        .i32()
        .ok_or(WasmExecutionError::ExportError(ExportError::IncompatibleType))?;

    // Check that the main function exists
    let _main: MainFunction = instance.exports.get_typed_function(store, main_fn)?;

    Ok(())
}
