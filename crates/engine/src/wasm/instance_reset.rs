//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Restoring a reused template instance to its freshly instantiated state.
//!
//! A transaction serves a template's calls from one instance (see
//! [`WasmInstanceCache`](super::WasmInstanceCache)), yet every call must start from exactly the
//! state a fresh instantiation gives it, so that no call can observe or influence another through
//! the guest. The WebAssembly spec enumerates everything an instance can change about itself — its
//! linear memory (contents and size), its mutable globals, its tables, and whether its data and
//! element segments have been dropped — and each is made restorable or immutable here:
//!
//! * **Memory** is restored: zero-filled, shrunk back to the module's minimum, and rewritten with the image
//!   instantiation left in it (the active data segments), captured once per loaded template.
//! * **Mutable globals** are restored: [`ResettableState`] exports every one the module defines, so the engine can read
//!   each initial value and write it back.
//! * **Tables** are immutable: `table.set`, `table.grow`, `table.fill` and `table.copy` are refused at compile.
//! * **Segments** are stateless: passive segments are refused, so every segment is dropped at instantiation and
//!   `memory.init`, `table.init`, `data.drop` and `elem.drop` cannot change anything.
//!
//! The globals the metering middlewares add are not restored and need not be: they are appended
//! after [`ResettableState`] runs, the engine sets the meter before every call, and the bulk
//! metering scratch globals are written before every read.

use wasmer::{
    AsStoreMut,
    ExportError,
    ExportIndex,
    Extern,
    Instance,
    LocalFunctionIndex,
    MemoryAccessError,
    MemoryError,
    ModuleInfo,
    Mutability,
    Pages,
    RuntimeError,
    Type,
    Value,
    sys::{FunctionMiddleware, MiddlewareError, MiddlewareReaderState, ModuleMiddleware},
    wasmparser::Operator,
};

const MIDDLEWARE_NAME: &str = "resettable_state";

/// Prefix of the export names [`ResettableState`] gives a module's mutable globals.
const GLOBAL_EXPORT_PREFIX: &str = "__tari_global_";

/// Zero runs shorter than this are kept inside the surrounding non-zero run of the memory image, so
/// a data segment with scattered zero bytes restores as one write rather than many.
const MIN_IMAGE_GAP: usize = 64;

/// Compile middleware that makes every piece of a template instance's state restorable or
/// immutable. It must be pushed before the metering middlewares, so the globals it exports are
/// exactly the module's own.
#[derive(Debug, Default)]
pub struct ResettableState;

impl ModuleMiddleware for ResettableState {
    fn generate_function_middleware<'a>(&self, _: LocalFunctionIndex) -> Box<dyn FunctionMiddleware<'a> + 'a> {
        Box::new(FunctionResettableState)
    }

    fn transform_module_info(&self, module_info: &mut ModuleInfo) -> Result<(), MiddlewareError> {
        if !module_info.passive_data.is_empty() {
            return Err(refused("passive data segments are not allowed"));
        }
        if !module_info.passive_elements.is_empty() {
            return Err(refused("passive element segments are not allowed"));
        }

        let defined_globals = module_info
            .globals
            .iter()
            .skip(module_info.num_imported_globals)
            .filter(|(_, global)| global.mutability == Mutability::Var)
            .map(|(index, global)| (index, global.ty));
        for (index, ty) in defined_globals {
            // A reference is only meaningful in the store that created it, so a reference-typed
            // global's initial value could not be written back into another instance's store.
            if !matches!(ty, Type::I32 | Type::I64 | Type::F32 | Type::F64) {
                return Err(refused(format!("mutable global {} has type {ty}", index.as_u32())));
            }
            let name = format!("{GLOBAL_EXPORT_PREFIX}{}", index.as_u32());
            if module_info.exports.contains_key(&name) {
                return Err(refused(format!("export name `{name}` is reserved")));
            }
            module_info.exports.insert(name, ExportIndex::Global(index));
        }
        Ok(())
    }
}

fn refused(message: impl Into<String>) -> MiddlewareError {
    MiddlewareError::new(MIDDLEWARE_NAME, message)
}

#[derive(Debug)]
struct FunctionResettableState;

impl<'a> FunctionMiddleware<'a> for FunctionResettableState {
    fn feed(&mut self, operator: Operator<'a>, state: &mut MiddlewareReaderState<'a>) -> Result<(), MiddlewareError> {
        match operator {
            Operator::TableSet { .. } |
            Operator::TableGrow { .. } |
            Operator::TableFill { .. } |
            Operator::TableCopy { .. } => Err(refused(format!("operator {operator:?} modifies a table"))),
            operator => {
                state.push_operator(operator);
                Ok(())
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InstanceResetError {
    #[error("guest memory: {0}")]
    Memory(#[from] MemoryError),
    #[error("guest memory access: {0}")]
    MemoryAccess(#[from] MemoryAccessError),
    #[error("instance export: {0}")]
    Export(#[from] ExportError),
    #[error("guest global: {0}")]
    Global(#[from] RuntimeError),
    #[error("guest memory is {actual:?} after a reset, not the {expected:?} it was instantiated with")]
    MemorySize { expected: Pages, actual: Pages },
    #[error("global `{name}` holds a value of type {ty}")]
    UnsupportedGlobal { name: String, ty: Type },
}

/// Everything a freshly instantiated template instance holds that the guest can change, captured
/// once per loaded template and written back into a reused instance by [`Self::restore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialState {
    memory_pages: Pages,
    /// The non-zero runs of the memory's contents, by offset. Everything outside them is zero.
    memory_image: Vec<(u64, Box<[u8]>)>,
    globals: Vec<(String, GlobalValue)>,
}

/// A numeric global's value, held as bits so a float round-trips exactly, NaN payload included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlobalValue {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
}

impl GlobalValue {
    fn from_value(name: &str, value: &Value) -> Result<Self, InstanceResetError> {
        match value {
            Value::I32(v) => Ok(Self::I32(*v)),
            Value::I64(v) => Ok(Self::I64(*v)),
            Value::F32(v) => Ok(Self::F32(v.to_bits())),
            Value::F64(v) => Ok(Self::F64(v.to_bits())),
            other => Err(InstanceResetError::UnsupportedGlobal {
                name: name.to_string(),
                ty: other.ty(),
            }),
        }
    }

    fn to_value(self) -> Value {
        match self {
            Self::I32(v) => Value::I32(v),
            Self::I64(v) => Value::I64(v),
            Self::F32(v) => Value::F32(f32::from_bits(v)),
            Self::F64(v) => Value::F64(f64::from_bits(v)),
        }
    }
}

impl InitialState {
    /// Reads the state of `instance`, which must be freshly instantiated from a module compiled
    /// with [`ResettableState`] and have run no code.
    pub fn capture(store: &mut impl AsStoreMut, instance: &Instance) -> Result<Self, InstanceResetError> {
        let view = instance.exports.get_memory("memory")?.view(store);
        let memory_pages = view.size();
        let memory_image = non_zero_runs(&view.copy_to_vec()?);

        let mut globals = Vec::new();
        for (name, export) in instance.exports.iter() {
            if !name.starts_with(GLOBAL_EXPORT_PREFIX) {
                continue;
            }
            if let Extern::Global(global) = export {
                globals.push((name.clone(), GlobalValue::from_value(name, &global.get(store))?));
            }
        }

        Ok(Self {
            memory_pages,
            memory_image,
            globals,
        })
    }

    /// Returns `instance`, an instance of the module this state was captured from, to that state.
    ///
    /// On error the instance is in an unknown state and must not run again.
    pub fn restore(&self, store: &mut impl AsStoreMut, instance: &Instance) -> Result<(), InstanceResetError> {
        let memory = instance.exports.get_memory("memory")?;
        // The engine's static memories are `PooledLinearMemory`, whose reset zero-fills every page
        // and returns the memory to the module's minimum size and protection boundary.
        memory.reset(store)?;
        let view = memory.view(store);
        if view.size() != self.memory_pages {
            return Err(InstanceResetError::MemorySize {
                expected: self.memory_pages,
                actual: view.size(),
            });
        }
        for (offset, bytes) in &self.memory_image {
            view.write(*offset, bytes)?;
        }

        for (name, value) in &self.globals {
            instance.exports.get_global(name)?.set(store, value.to_value())?;
        }
        Ok(())
    }
}

/// The non-zero runs of `memory`, merging runs separated by fewer than [`MIN_IMAGE_GAP`] zeroes.
fn non_zero_runs(memory: &[u8]) -> Vec<(u64, Box<[u8]>)> {
    let mut runs = Vec::new();
    let mut offset = 0;
    while let Some(start) = memory[offset..].iter().position(|&b| b != 0).map(|p| p + offset) {
        let mut end = start;
        let mut zeroes = 0;
        for (i, &b) in memory[start..].iter().enumerate() {
            if b == 0 {
                zeroes += 1;
                if zeroes >= MIN_IMAGE_GAP {
                    break;
                }
            } else {
                zeroes = 0;
                end = start + i + 1;
            }
        }
        runs.push((start as u64, memory[start..end].into()));
        offset = end;
    }
    runs
}

#[cfg(test)]
mod tests {
    use tari_template_builtin::{
        ACCOUNT_TEMPLATE_ADDRESS,
        LIQUIDITY_POOL_TEMPLATE_ADDRESS,
        NFT_FAUCET_TEMPLATE_ADDRESS,
        XTR_FAUCET_TEMPLATE_ADDRESS,
        get_template_builtin,
    };
    use wasmer::{Function, Module, Store, TypedFunction, imports};

    use super::*;
    use crate::{template::LoadedTemplate, wasm::WasmModule};

    fn compile(wat: &str) -> Result<(Store, Module), wasmer::CompileError> {
        let store = Store::new(super::super::engine::create_engine());
        let module = Module::new(&store, wat::parse_str(wat).unwrap())?;
        Ok((store, module))
    }

    fn refusal(wat: &str) -> String {
        match compile(wat) {
            Ok(_) => panic!("module compiled"),
            Err(err) => err.to_string(),
        }
    }

    /// A module whose one function changes every piece of state an instance can hold that the
    /// engine lets it change: it bumps a mutable global, overwrites the data segment, writes past
    /// it and grows the memory.
    const STATEFUL: &str = r#"
        (module
          (memory (export "memory") 1 8)
          (global $counter (mut i32) (i32.const 7))
          (global $wide (mut f64) (f64.const nan:0x4000000000001))
          (data (i32.const 16) "initial")
          (func (export "mutate")
            (global.set $counter (i32.add (global.get $counter) (i32.const 1)))
            (global.set $wide (f64.const 1.5))
            (i64.store (i32.const 16) (i64.const -1))
            (i32.store (i32.const 40000) (i32.const 3))
            (drop (memory.grow (i32.const 2)))
            (i32.store (i32.const 150000) (i32.const 9))))
    "#;

    #[test]
    fn a_restored_instance_matches_a_fresh_one() {
        let (mut store, module) = compile(STATEFUL).unwrap();
        let instance = Instance::new(&mut store, &module, &imports! {}).unwrap();
        let initial = InitialState::capture(&mut store, &instance).unwrap();
        assert_eq!(initial.globals.len(), 2, "both mutable globals are exported");

        let mutate: TypedFunction<(), ()> = instance.exports.get_typed_function(&store, "mutate").unwrap();
        mutate.call(&mut store).unwrap();
        mutate.call(&mut store).unwrap();
        assert_ne!(InitialState::capture(&mut store, &instance).unwrap(), initial);

        initial.restore(&mut store, &instance).unwrap();
        assert_eq!(InitialState::capture(&mut store, &instance).unwrap(), initial);

        let fresh = Instance::new(&mut store, &module, &imports! {}).unwrap();
        assert_eq!(InitialState::capture(&mut store, &fresh).unwrap(), initial);
    }

    #[test]
    fn a_restored_builtin_template_matches_its_fresh_instance() {
        for address in [
            ACCOUNT_TEMPLATE_ADDRESS,
            XTR_FAUCET_TEMPLATE_ADDRESS,
            NFT_FAUCET_TEMPLATE_ADDRESS,
            LIQUIDITY_POOL_TEMPLATE_ADDRESS,
        ] {
            let LoadedTemplate::Wasm(template) =
                WasmModule::load_template_from_code(get_template_builtin(&address)).unwrap();
            let mut store = template.create_store();
            let imports = imports! {
                "env" => {
                    "tari_engine" => Function::new_typed(&mut store, |_: i32, _: i32, _: i32| 0i32),
                    "on_panic" => Function::new_typed(&mut store, |_: i32, _: i32, _: i32, _: i32| {}),
                }
            };
            let instance = Instance::new(&mut store, template.wasm_module(), &imports).unwrap();
            assert_eq!(
                &InitialState::capture(&mut store, &instance).unwrap(),
                template.initial_state(),
                "a fresh instance of {address} must match the state captured at load"
            );
            assert!(
                !template.initial_state().globals.is_empty(),
                "{address} exposes its stack pointer"
            );

            // Churn the allocator's statics and heap far enough to grow the memory, then scribble
            // over every accessible byte and every restorable global.
            let alloc: TypedFunction<i32, i32> = instance.exports.get_typed_function(&store, "tari_alloc").unwrap();
            let free: TypedFunction<i32, ()> = instance.exports.get_typed_function(&store, "tari_free").unwrap();
            let memory = instance.exports.get_memory("memory").unwrap();
            let initial_pages = template.initial_state().memory_pages;
            for i in 0..256 {
                if memory.view(&store).size() > initial_pages {
                    break;
                }
                let ptr = alloc.call(&mut store, 16 * 1024).unwrap();
                // A failed allocation still returns the address past its length prefix.
                assert!(ptr > 4, "{address}: allocation failed before the memory grew");
                if i % 2 == 1 {
                    free.call(&mut store, ptr).unwrap();
                }
            }
            assert!(
                memory.view(&store).size() > initial_pages,
                "{address}: the memory never grew"
            );
            let size = memory.view(&store).data_size();
            memory.view(&store).write(0, &vec![0xA5; size as usize]).unwrap();
            for (name, initial) in &template.initial_state().globals {
                let garbage = match initial {
                    GlobalValue::I32(v) => Value::I32(v ^ 0x7fff_fff0),
                    GlobalValue::I64(v) => Value::I64(v ^ 0x7fff_fff0),
                    GlobalValue::F32(v) => Value::F32(f32::from_bits(v ^ 0x7fff_fff0)),
                    GlobalValue::F64(v) => Value::F64(f64::from_bits(v ^ 0x7fff_fff0)),
                };
                instance
                    .exports
                    .get_global(name)
                    .unwrap()
                    .set(&mut store, garbage)
                    .unwrap();
            }

            template.initial_state().restore(&mut store, &instance).unwrap();
            assert_eq!(
                &InitialState::capture(&mut store, &instance).unwrap(),
                template.initial_state(),
                "a restored instance of {address} must match its fresh state"
            );
        }
    }

    #[test]
    fn table_modifying_operators_are_refused() {
        for op in [
            "(table.set (i32.const 0) (ref.null func))",
            "(drop (table.grow (ref.null func) (i32.const 1)))",
            "(table.fill (i32.const 0) (ref.null func) (i32.const 1))",
            "(table.copy (i32.const 0) (i32.const 0) (i32.const 1))",
        ] {
            let wat = format!(r#"(module (table 1 4 funcref) (func {op}))"#);
            assert!(refusal(&wat).contains("modifies a table"), "{op} was not refused");
        }
    }

    #[test]
    fn passive_segments_are_refused() {
        assert!(refusal(r#"(module (memory 1) (data "passive"))"#).contains("passive data"));
        assert!(refusal(r#"(module (func $f) (elem func $f))"#).contains("passive element"));
    }

    #[test]
    fn a_reference_typed_mutable_global_is_refused() {
        assert!(refusal(r#"(module (global (mut funcref) (ref.null func)))"#).contains("mutable global"));
    }

    #[test]
    fn an_immutable_global_and_a_declared_segment_are_accepted() {
        compile(
            r#"(module
                 (global i32 (i32.const 1))
                 (func $f)
                 (elem declare func $f)
                 (func (drop (ref.func $f))))"#,
        )
        .unwrap();
    }

    #[test]
    fn zero_gaps_shorter_than_the_threshold_stay_in_one_run() {
        let mut memory = vec![0u8; 1024];
        memory[10] = 1;
        memory[20] = 2;
        memory[20 + MIN_IMAGE_GAP + 1] = 3;
        let runs = non_zero_runs(&memory);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].0, 10);
        assert_eq!(runs[0].1.len(), 11);
        assert_eq!(runs[1].0, (21 + MIN_IMAGE_GAP) as u64);
        assert_eq!(&*runs[1].1, &[3]);
    }
}
