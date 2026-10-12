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

//! Derivation of the [`ModuleShape`] counts that price instantiation, and the structural limits
//! checked alongside them.
//!
//! Its own file because [`ENGINE_FINGERPRINT`](super::cache::ENGINE_FINGERPRINT) hashes this
//! source. A cache hit serves these counts verbatim out of the file header without re-deriving
//! them, and they price `instantiation_points` into a committed fee receipt, so a node that
//! counted differently when it wrote the file charges a different fee than one compiling fresh.
//! The limits enforced here are in the same position: `max_tables`, `max_globals`, the module and
//! function limits on what the compile emits, and the cap on the compile work they add up to are
//! checked at compile time only, and a hit never reaches them.

use tari_engine_types::limits::{self, CompileCounts, ModuleShape};
use tari_wasmer_middlewares::metering::is_accounting;
use wasmer::wasmparser::{DataKind, ElementItems, ElementKind, Operator, OperatorsReader, Parser, Payload, TypeRef};

use crate::wasm::{WasmValidationError, bulk_metering};

/// What [`validate_module_structure`] reads out of a module before it is compiled.
#[derive(Debug, Clone, Copy)]
pub struct ModuleStructure {
    /// The counts that price each instantiation.
    pub shape: ModuleShape,
    /// The functions, variables and metering checks the compile emits code for.
    pub compile: CompileCounts,
}

/// Checks what only the module bytes show: that the module declares no start function, no more
/// tables, globals, functions or variables than the limits, no function whose control flow or
/// metering is larger than the function limits, and no more compile work than a binary at the
/// size cap.
///
/// A start function runs on every instantiation, before the engine has installed this call's
/// metering allowance and outside any invocation it could attribute effects to. Templates have no
/// use for one: the engine only ever enters a template through its `<name>_main` export.
///
/// Tables and globals are both host storage built at every instantiation and claimed by a
/// declaration far smaller than what it claims. Each table's element count is bounded by the
/// tunables, which see one table at a time, so the number of tables is what bounds the storage all
/// of them together claim; a global's slot is fixed, so its count is the whole bound.
///
/// Functions, the parameters and locals they declare and the metering checks compiled into them
/// are bounded because the compile's cost for each is far above what the bytes asking for it are
/// priced at, and the compile runs before anything else could refuse the module.
pub(crate) fn validate_module_structure(code: &[u8]) -> Result<ModuleStructure, WasmValidationError> {
    let mut shape = ModuleShape::default();
    let mut compile = CompileCounts::default();
    // Parameter count of each signature in the type section, which precedes every section that
    // refers to it.
    let mut type_params = Vec::new();
    // Imported functions take the lowest function indexes, ahead of every function the code
    // section defines.
    let mut imported_functions = 0u32;
    let mut defined_functions = 0u32;
    let mut loops = 0usize;
    for payload in Parser::new(0).parse_all(code) {
        // Malformed wasm: stop and let the cranelift compile in
        // `load_template_from_code` report the canonical CompileError.
        let Ok(payload) = payload else { break };
        match payload {
            Payload::StartSection { .. } => return Err(WasmValidationError::StartSectionNotAllowed),
            Payload::TypeSection(reader) => {
                for ty in reader.into_iter_err_on_gc_types().flatten() {
                    type_params.push(ty.params().len() as u64);
                }
            },
            Payload::ImportSection(reader) => {
                for import in reader.into_imports().flatten() {
                    if let TypeRef::Func(ty) | TypeRef::FuncExact(ty) = import.ty {
                        add_function(&mut compile, &type_params, ty)?;
                        imported_functions = imported_functions.saturating_add(1);
                    }
                }
            },
            Payload::FunctionSection(reader) => {
                for ty in reader.into_iter().flatten() {
                    add_function(&mut compile, &type_params, ty)?;
                }
            },
            Payload::CodeSectionEntry(body) => {
                let function = imported_functions.saturating_add(defined_functions);
                defined_functions = defined_functions.saturating_add(1);
                let Ok(locals) = body.get_locals_reader() else { break };
                for (count, _) in locals.into_iter().flatten() {
                    add_variables(&mut compile, u64::from(count))?;
                }
                let Ok(operators) = body.get_operators_reader() else {
                    break;
                };
                loops = loops.saturating_add(add_function_body(&mut compile, function, operators)?);
                let max_loops = limits::WASM_LIMITS.max_module_loops;
                if loops > max_loops {
                    return Err(WasmValidationError::TooManyModuleLoops { max_loops });
                }
            },
            Payload::TableSection(reader) => {
                let count = reader.count() as usize;
                if count > limits::WASM_LIMITS.max_tables {
                    return Err(WasmValidationError::TooManyTables {
                        count,
                        max_tables: limits::WASM_LIMITS.max_tables,
                    });
                }
                for table in reader.into_iter().flatten() {
                    shape.declared_table_slots = shape.declared_table_slots.saturating_add(table.ty.initial);
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
            // Only active segments are written into the instance's storage when it is built. A
            // passive segment stays in the module until a `memory.init` or `table.init` reaches for
            // it, so its bytes are a cost of that operator rather than of instantiation, and a call
            // that never executes one must not pay for it.
            Payload::DataSection(reader) => {
                for segment in reader.into_iter().flatten() {
                    if matches!(segment.kind, DataKind::Active { .. }) {
                        shape.data_segment_bytes = shape.data_segment_bytes.saturating_add(segment.data.len() as u64);
                        shape.data_segment_count = shape.data_segment_count.saturating_add(1);
                    }
                }
            },
            Payload::ElementSection(reader) => {
                for segment in reader.into_iter().flatten() {
                    if !matches!(segment.kind, ElementKind::Active { .. }) {
                        continue;
                    }
                    let entries = match segment.items {
                        ElementItems::Functions(items) => u64::from(items.count()),
                        ElementItems::Expressions(_, items) => u64::from(items.count()),
                    };
                    shape.element_segment_entries = shape.element_segment_entries.saturating_add(entries);
                }
            },
            _ => {},
        }
    }
    let billed_bytes = limits::template_compile_count_bytes(&compile);
    let max_bytes = limits::ENGINE_LIMITS.max_template_binary_size_bytes;
    if billed_bytes > max_bytes as u64 {
        return Err(WasmValidationError::TooMuchCompileWork {
            billed_bytes,
            max_bytes,
        });
    }
    Ok(ModuleStructure { shape, compile })
}

/// Counts a function of signature `ty` and its parameters. A signature the type section does not
/// declare leaves the module for the compile to refuse.
fn add_function(compile: &mut CompileCounts, type_params: &[u64], ty: u32) -> Result<(), WasmValidationError> {
    compile.functions = compile.functions.saturating_add(1);
    let max_functions = limits::WASM_LIMITS.max_module_functions;
    if compile.functions > max_functions as u64 {
        return Err(WasmValidationError::TooManyModuleFunctions { max_functions });
    }
    let params = type_params.get(ty as usize).copied().unwrap_or(0);
    add_variables(compile, params)
}

/// Counts the metering checks the compile emits into function `function`, refusing it for more of
/// them, more `loop`s or deeper nesting than the function limits allow. Returns its `loop`s.
///
/// A check follows every operator the metering middleware ends a basic block at and every operator
/// the bulk meter charges by length, so those are the operators counted. A body that does not
/// decode is left for the compile to refuse.
fn add_function_body(
    compile: &mut CompileCounts,
    function: u32,
    mut operators: OperatorsReader<'_>,
) -> Result<usize, WasmValidationError> {
    let max_checks = limits::WASM_LIMITS.max_function_metering_checks;
    let max_loops = limits::WASM_LIMITS.max_function_loops;
    let max_depth = limits::WASM_LIMITS.max_function_nesting_depth;
    let (mut checks, mut loops, mut depth) = (0usize, 0usize, 0usize);
    while !operators.eof() {
        let Ok(operator) = operators.read() else { break };
        match operator {
            Operator::Loop { .. } => {
                loops += 1;
                if loops > max_loops {
                    return Err(WasmValidationError::TooManyFunctionLoops { function, max_loops });
                }
                depth += 1;
            },
            Operator::Block { .. } | Operator::If { .. } => depth += 1,
            // The function body's own closing `end` has no block to close.
            Operator::End => depth = depth.saturating_sub(1),
            _ => {},
        }
        if depth > max_depth {
            return Err(WasmValidationError::FunctionNestedTooDeep { function, max_depth });
        }
        if is_accounting(&operator) || bulk_metering::charge_for(&operator).is_some() {
            checks += 1;
            if checks > max_checks {
                return Err(WasmValidationError::TooManyFunctionMeteringChecks { function, max_checks });
            }
        }
    }
    let checks = checks as u64;
    compile.metering_checks = compile.metering_checks.saturating_add(checks);
    compile.largest_function_metering_checks = compile.largest_function_metering_checks.max(checks);
    Ok(loops)
}

fn add_variables(compile: &mut CompileCounts, count: u64) -> Result<(), WasmValidationError> {
    compile.variables = compile.variables.saturating_add(count);
    let max_variables = limits::WASM_LIMITS.max_module_variables;
    if compile.variables > max_variables as u64 {
        return Err(WasmValidationError::TooManyModuleVariables { max_variables });
    }
    Ok(())
}
