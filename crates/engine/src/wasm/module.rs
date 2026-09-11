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

use tari_engine_types::limits;
use tari_template_abi::{FunctionDef, TEMPLATE_DEF_CUSTOM_SECTION, TemplateDef, Type, WASM_PTR_SIZE};
use wasmer::{
    Engine,
    ExternType,
    Function,
    FunctionType,
    Instance,
    Pages,
    Store,
    TypedFunction,
    WasmPtr,
    imports,
    sys::{BaseTunables, CompilerConfig, Cranelift, CraneliftOptLevel, EngineBuilder},
    wasmparser::{Parser, Payload},
};

use crate::{
    template::{LoadedTemplate, TemplateLoaderError, TemplateModuleLoader},
    wasm::{WasmExecutionError, WasmProcess, WasmValidationError, limiting_tunable::LimitingTunables, metering},
};

pub type MainFunction = TypedFunction<(WasmPtr<u8>, u32), WasmPtr<u8>>;
#[derive(Debug, Clone)]
pub struct WasmModule {
    code: Box<[u8]>,
}

impl WasmModule {
    pub fn from_code(code: impl Into<Box<[u8]>>) -> Self {
        Self { code: code.into() }
    }

    pub fn validate_code(code: &[u8]) -> Result<TemplateDef, TemplateLoaderError> {
        // Admission rule for externally-published templates: reject custom
        // sections the engine does not consume before paying for the cranelift
        // compile below. Only the registration path runs this; already-stored
        // templates (and built-ins) load via `load_template_from_code` without
        // it.
        reject_disallowed_custom_sections(code).map_err(WasmExecutionError::from)?;
        // TODO: evaluate if there are acceptable cheaper ways to fully validate
        let loaded = Self::load_template_from_code(code)?;
        Ok(loaded.into_template_def())
    }

    pub fn load_template_from_code(code: &[u8]) -> Result<LoadedTemplate, TemplateLoaderError> {
        validate_module_structure(code).map_err(WasmExecutionError::from)?;
        let engine = Self::create_engine();
        let module = wasmer::Module::new(&engine, code)?;
        Self::finalize_loaded_module(engine, module, code.len())
    }

    /// Load a template from a previously serialized wasmer module (see
    /// [`wasmer::Module::serialize`]). `code_size` is the size of the original
    /// WASM source bytes — preserved from the source compile and used by
    /// downstream caches (e.g. the in-memory moka weigher in
    /// `MemoryCacheTemplateProvider`).
    ///
    /// Takes [`bytes::Bytes`] so callers can pass mmap-backed regions through
    /// without a copy: `wasmer::Module::deserialize_unchecked` accepts `Bytes`
    /// directly, and [`bytes::Bytes::from_owner`] wraps any
    /// `AsRef<[u8]> + Send + 'static` (such as [`memmap2::Mmap`]) without
    /// copying. With `&[u8]`, wasmer's `IntoBytes` impl falls back to
    /// `to_vec()` and we'd pay an extra full-artifact allocation on every
    /// cache hit.
    ///
    /// # Safety
    ///
    /// The serialized bytes MUST have been produced by `wasmer::Module::serialize`
    /// against an engine configured identically to [`Self::create_engine`]. Feeding
    /// arbitrary bytes here is undefined behaviour. Callers are expected to gate
    /// this behind a node-local cache directory whose contents only this process
    /// writes.
    #[cfg(feature = "wasm-cache")]
    pub unsafe fn load_template_from_serialized(
        serialized: bytes::Bytes,
        code_size: usize,
    ) -> Result<LoadedTemplate, TemplateLoaderError> {
        let engine = Self::create_engine();
        // SAFETY: forwarded to caller — see function-level docs.
        let module = unsafe { wasmer::Module::deserialize_unchecked(&engine, serialized) }?;
        Self::finalize_loaded_module(engine, module, code_size)
    }

    /// Validates a compiled module and turns it into a [`LoadedTemplate`].
    ///
    /// Every check the module itself can answer runs before the module is instantiated.
    /// Instantiating creates the guest's linear memory and tables, sized by values the module
    /// declares, so a module reaches it only once the engine has accepted its ABI and its exports.
    fn finalize_loaded_module(
        engine: Engine,
        module: wasmer::Module,
        code_size: usize,
    ) -> Result<LoadedTemplate, TemplateLoaderError> {
        let template = load_template_def_from_custom_section(&module)?;
        let main_fn = format!("{}_main", template.template_name());

        WasmProcess::validate_template_abi_version(&template)?;
        validate_functions(&template)?;
        validate_module_exports(&module, &main_fn)?;

        let mut store = Store::new(engine);
        let imports = imports! {
            "env" => {
                "tari_engine" => Function::new_typed(&mut store, |_op: i32, _arg_ptr: i32, _arg_len: i32| 0i32),
                "tari_debug" => Function::new_typed(&mut store, |_arg_ptr: i32, _arg_len: i32| {  }),
                "on_panic" => Function::new_typed(&mut store, |_msg_ptr: i32, _msg_len: i32, _line: i32, _col: i32| {  }),
            }
        };
        // The memory and table limits live in [`LimitingTunables`], which only sees a module's
        // declared types when the instance's storage is created. Instantiating here is what applies
        // them, so a template that declares more than a limit allows is refused at load rather than
        // at its first call.
        Instance::new(&mut store, &module, &imports)?;

        let engine = store.engine().clone();

        Ok(LoadedWasmTemplate::new(template, module, engine, code_size).into())
    }

    pub fn code(&self) -> &[u8] {
        &self.code
    }

    pub fn into_code(self) -> Box<[u8]> {
        self.code
    }

    fn create_engine() -> Engine {
        const MEMORY_PAGE_LIMIT: Pages = Pages(limits::WASM_LIMITS.max_memory_pages as u32);
        let base = BaseTunables::new();
        let tunables = LimitingTunables::new(base, MEMORY_PAGE_LIMIT, limits::WASM_LIMITS.max_table_elements);
        let mut compiler = Cranelift::new();
        compiler
            .opt_level(CraneliftOptLevel::SpeedAndSize)
            .canonicalize_nans(true);
        // Per-call metering ceiling. `WasmProcess::invoke` lowers each call's allowance further to
        // whatever remains of the per-transaction budget (`MAX_WASM_POINTS_PER_TRANSACTION`).
        compiler.push_middleware(Arc::new(metering::middleware(limits::MAX_WASM_POINTS_PER_CALL)));

        // Every feature is set explicitly rather than relying on `Features::default()`: the
        // accepted-module set is consensus-critical, and wasmer flips defaults between releases
        // (e.g. `extended_const` became default-on in 7.1.0). When bumping wasmer, add any newly
        // introduced feature flag here explicitly.
        let mut features = wasmer::sys::Features::default();
        features
            .threads(false)
            .bulk_memory(true)
            .multi_value(false)
            .reference_types(true)
            .simd(false)
            .relaxed_simd(false)
            .tail_call(false)
            .memory64(false)
            .multi_memory(false)
            .exceptions(false)
            .module_linking(false)
            .extended_const(false)
            .wide_arithmetic(false);

        let mut engine = EngineBuilder::new(compiler).set_features(Some(features)).engine();
        engine.set_tunables(tunables);
        Engine::from(engine)
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

    pub fn into_template_def(self) -> TemplateDef {
        Arc::try_unwrap(self.template_def).unwrap_or_else(|arc| (*arc).clone())
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

/// Recovers the `TemplateDef` from the `tari_tdef` custom section, which every template must
/// carry. The section is part of the module, so reading it needs no instance and no access to guest
/// memory.
///
/// Wasmer preserves custom sections through compile and `serialize` /
/// `deserialize`, so this works on both freshly compiled modules and modules
/// loaded from the disk cache.
fn load_template_def_from_custom_section(module: &wasmer::Module) -> Result<TemplateDef, WasmExecutionError> {
    let mut sections = module.custom_sections(TEMPLATE_DEF_CUSTOM_SECTION);
    let Some(section) = sections.next() else {
        return Err(WasmExecutionError::AbiTemplateDefSectionMissing);
    };
    // The macro emits exactly one `tari_tdef` section per template. Multiple
    // sections with this name would be ambiguous — refuse to guess which one
    // is canonical.
    if sections.next().is_some() {
        return Err(WasmExecutionError::AbiTemplateDefSectionMalformed {
            reason: format!(
                "module contains more than one `{}` custom section",
                TEMPLATE_DEF_CUSTOM_SECTION
            ),
        });
    }
    if section.len() < WASM_PTR_SIZE {
        return Err(WasmExecutionError::AbiTemplateDefSectionMalformed {
            reason: format!(
                "section is {} bytes; expected at least {} for the length prefix",
                section.len(),
                WASM_PTR_SIZE
            ),
        });
    }
    let prefix: [u8; WASM_PTR_SIZE] = section[..WASM_PTR_SIZE]
        .try_into()
        .expect("section.len() >= WASM_PTR_SIZE checked above");
    let full_len = u32::from_le_bytes(prefix) as usize;
    if full_len < WASM_PTR_SIZE || full_len > section.len() {
        return Err(WasmExecutionError::AbiTemplateDefSectionMalformed {
            reason: format!(
                "declared length {} is inconsistent with section size {}",
                full_len,
                section.len()
            ),
        });
    }
    let template = tari_bor::decode::<TemplateDef>(&section[WASM_PTR_SIZE..full_len])
        .map_err(WasmExecutionError::AbiTemplateDefDecodeError)?;
    Ok(template)
}

/// Custom sections the engine consumes and therefore admits into a published
/// template. Everything else (DWARF `.debug_*`, the `name` section,
/// `producers`, …) is semantically inert — cranelift ignores it — but is stored
/// verbatim with the template and replicated across the whole validator
/// committee, so it is rejected at registration.
const ALLOWED_CUSTOM_SECTIONS: &[&str] = &[TEMPLATE_DEF_CUSTOM_SECTION];

/// Reject a published template that carries any custom section other than those
/// in [`ALLOWED_CUSTOM_SECTIONS`].
///
/// The scan defers to the cranelift compile in
/// [`WasmModule::load_template_from_code`] for malformed input: a binary that
/// `wasmparser` cannot parse will also fail wasmer's validation, so stopping at
/// the first parse error lets that path report the canonical `CompileError`
/// rather than a less precise one here.
fn reject_disallowed_custom_sections(code: &[u8]) -> Result<(), WasmValidationError> {
    for payload in Parser::new(0).parse_all(code) {
        // Malformed wasm: stop and let the cranelift compile in
        // `load_template_from_code` report the canonical CompileError.
        let Ok(payload) = payload else { break };
        if let Payload::CustomSection(reader) = payload &&
            !ALLOWED_CUSTOM_SECTIONS.contains(&reader.name())
        {
            return Err(WasmValidationError::DisallowedCustomSection {
                name: reader.name().to_string(),
            });
        }
    }
    Ok(())
}

/// Checks a module's exports against the template ABI, reading them off the module rather than an
/// instance: the memory the engine reads and writes, the three functions it calls and their
/// signatures, and that the module exports no other function.
fn validate_module_exports(module: &wasmer::Module, main_fn: &str) -> Result<(), WasmExecutionError> {
    // `(call_info_ptr: i32, call_info_len: i32) -> i32`, matching [`MainFunction`].
    let expected_main = FunctionType::new([wasmer::Type::I32, wasmer::Type::I32], [wasmer::Type::I32]);
    // `(len: i32) -> i32` and `(ptr: i32)`, matching `WasmAllocFn` and `WasmFreeFn`.
    let expected_alloc = FunctionType::new([wasmer::Type::I32], [wasmer::Type::I32]);
    let expected_free = FunctionType::new([wasmer::Type::I32], []);

    let mut memory_export = false;
    let mut main_signature = None;
    let mut alloc_signature = None;
    let mut free_signature = None;

    for export in module.exports() {
        match export.ty() {
            ExternType::Function(signature) => match export.name() {
                name if name == main_fn => main_signature = Some(signature.clone()),
                "tari_alloc" => alloc_signature = Some(signature.clone()),
                "tari_free" => free_signature = Some(signature.clone()),
                name => {
                    return Err(WasmExecutionError::UnexpectedAbiFunction { name: name.to_string() });
                },
            },
            ExternType::Memory(_) => {
                memory_export |= export.name() == "memory";
            },
            ExternType::Global(_) | ExternType::Table(_) | ExternType::Tag(_) => {},
        }
    }

    if !memory_export {
        return Err(WasmValidationError::MissingExport {
            name: "memory".to_string(),
        }
        .into());
    }

    validate_export_signature(main_fn, main_signature.as_ref(), &expected_main)?;
    validate_export_signature("tari_alloc", alloc_signature.as_ref(), &expected_alloc)?;
    validate_export_signature("tari_free", free_signature.as_ref(), &expected_free)?;

    Ok(())
}

/// The engine calls each ABI function by name and by type, so a module that exports one under
/// another signature — or not at all — is refused at admission rather than on its first call.
fn validate_export_signature(
    name: &str,
    signature: Option<&FunctionType>,
    expected: &FunctionType,
) -> Result<(), WasmValidationError> {
    match signature {
        Some(signature) if signature == expected => Ok(()),
        Some(signature) => Err(WasmValidationError::InvalidExportSignature {
            name: name.to_string(),
            signature: signature.to_string(),
            expected: expected.to_string(),
        }),
        None => Err(WasmValidationError::MissingExport { name: name.to_string() }),
    }
}

/// Checks what only the module bytes show: that the module declares no start function, and no more
/// tables or globals than the limits.
///
/// A start function runs on every instantiation, before the engine has installed this call's
/// metering allowance and outside any invocation it could attribute effects to. Templates have no
/// use for one: the engine only ever enters a template through its `<name>_main` export.
///
/// Tables and globals are both host storage built at every instantiation and claimed by a
/// declaration far smaller than what it claims. Each table's element count is bounded by the
/// tunables, which see one table at a time, so the number of tables is what bounds the storage all
/// of them together claim; a global's slot is fixed, so its count is the whole bound.
fn validate_module_structure(code: &[u8]) -> Result<(), WasmValidationError> {
    for payload in Parser::new(0).parse_all(code) {
        // Malformed wasm: stop and let the cranelift compile in
        // `load_template_from_code` report the canonical CompileError.
        let Ok(payload) = payload else { break };
        match payload {
            Payload::StartSection { .. } => return Err(WasmValidationError::StartSectionNotAllowed),
            Payload::TableSection(reader) => {
                let count = reader.count() as usize;
                if count > limits::WASM_LIMITS.max_tables {
                    return Err(WasmValidationError::TooManyTables {
                        count,
                        max_tables: limits::WASM_LIMITS.max_tables,
                    });
                }
            },
            Payload::GlobalSection(reader) => {
                let count = reader.count() as usize;
                if count > limits::WASM_LIMITS.max_globals {
                    return Err(WasmValidationError::TooManyGlobals {
                        count,
                        max_globals: limits::WASM_LIMITS.max_globals,
                    });
                }
            },
            _ => {},
        }
    }
    Ok(())
}

fn validate_functions(template_def: &TemplateDef) -> Result<(), WasmExecutionError> {
    match template_def {
        TemplateDef::V1(def) => {
            let function_count = def.functions.len();
            if function_count > limits::WASM_LIMITS.max_functions {
                return Err(WasmValidationError::TooManyFunctions {
                    max_functions: limits::WASM_LIMITS.max_functions,
                }
                .into());
            }
            for func in &def.functions {
                if func.name.len() > limits::WASM_LIMITS.max_function_name_length {
                    return Err(WasmValidationError::FunctionNameTooLong {
                        name: func.name.clone(),
                        max_length: limits::WASM_LIMITS.max_function_name_length,
                    }
                    .into());
                }

                if func.arguments.len() > limits::WASM_LIMITS.max_function_arguments {
                    return Err(WasmValidationError::FunctionTooManyArguments {
                        name: func.name.clone(),
                        max_args: limits::WASM_LIMITS.max_function_arguments,
                        num_args: func.arguments.len(),
                    }
                    .into());
                }
                for arg in &func.arguments {
                    if arg.name.len() > limits::WASM_LIMITS.max_function_name_length {
                        return Err(WasmValidationError::FunctionNameTooLong {
                            name: arg.name.clone(),
                            max_length: limits::WASM_LIMITS.max_function_name_length,
                        }
                        .into());
                    }
                    match &arg.arg_type {
                        Type::Tuple(tuple) if tuple.len() > limits::WASM_LIMITS.max_function_arguments => {
                            return Err(WasmValidationError::FunctionTooManyTupleReturn {
                                name: func.name.clone(),
                                max_tuple_size: limits::WASM_LIMITS.max_function_arguments,
                                tuple_size: tuple.len(),
                            }
                            .into());
                        },
                        Type::Other { name } if name.len() > limits::WASM_LIMITS.max_function_name_length => {
                            return Err(WasmValidationError::FunctionNameTooLong {
                                name: name.clone(),
                                max_length: limits::WASM_LIMITS.max_function_name_length,
                            }
                            .into());
                        },
                        _ => {},
                    }
                }
                if func.is_migration {
                    // Note that we are checking the TemplateDef, not the actual return type of the function in Wasm.
                    match &func.output {
                        Type::Other { name } => {
                            if name != "Self" &&
                                name != template_def.template_name() &&
                                name != "Component<Self>" &&
                                *name != format!("Component<{}>", template_def.template_name())
                            {
                                return Err(WasmValidationError::InvalidMigrationReturnType {
                                    function_name: func.name.clone(),
                                    return_type: func.output.clone(),
                                }
                                .into());
                            }
                        },
                        _ => {
                            return Err(WasmValidationError::InvalidMigrationReturnType {
                                function_name: func.name.clone(),
                                return_type: func.output.clone(),
                            }
                            .into());
                        },
                    }
                }
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_uleb128(out: &mut Vec<u8>, mut value: u32) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    /// Build a header-only WASM module carrying the given named custom sections,
    /// in order. `wasmparser` accepts magic+version plus custom sections, which
    /// is all `reject_disallowed_custom_sections` inspects.
    fn wasm_with_custom_sections(sections: &[(&str, &[u8])]) -> Vec<u8> {
        let mut wasm = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        for (name, payload) in sections {
            let mut body = Vec::new();
            write_uleb128(&mut body, name.len() as u32);
            body.extend_from_slice(name.as_bytes());
            body.extend_from_slice(payload);
            wasm.push(0); // custom section id
            write_uleb128(&mut wasm, body.len() as u32);
            wasm.extend_from_slice(&body);
        }
        wasm
    }

    #[test]
    fn accepts_module_without_custom_sections() {
        let wasm = wasm_with_custom_sections(&[]);
        reject_disallowed_custom_sections(&wasm).expect("no custom sections is allowed");
    }

    #[test]
    fn accepts_only_template_def_section() {
        let wasm = wasm_with_custom_sections(&[(TEMPLATE_DEF_CUSTOM_SECTION, &[1, 2, 3, 4])]);
        reject_disallowed_custom_sections(&wasm).expect("tari_tdef is allowed");
    }

    #[test]
    fn rejects_name_section() {
        let wasm = wasm_with_custom_sections(&[("name", &[0u8; 64])]);
        match reject_disallowed_custom_sections(&wasm) {
            Err(WasmValidationError::DisallowedCustomSection { name }) => assert_eq!(name, "name"),
            other => panic!("expected DisallowedCustomSection, got {other:?}"),
        }
    }

    #[test]
    fn rejects_dwarf_debug_section() {
        let wasm = wasm_with_custom_sections(&[(".debug_info", &[0u8; 1024])]);
        match reject_disallowed_custom_sections(&wasm) {
            Err(WasmValidationError::DisallowedCustomSection { name }) => assert_eq!(name, ".debug_info"),
            other => panic!("expected DisallowedCustomSection, got {other:?}"),
        }
    }

    #[test]
    fn rejects_disallowed_section_alongside_template_def() {
        let wasm =
            wasm_with_custom_sections(&[(TEMPLATE_DEF_CUSTOM_SECTION, &[1, 2, 3, 4]), ("producers", &[0u8; 16])]);
        match reject_disallowed_custom_sections(&wasm) {
            Err(WasmValidationError::DisallowedCustomSection { name }) => assert_eq!(name, "producers"),
            other => panic!("expected DisallowedCustomSection, got {other:?}"),
        }
    }
}
