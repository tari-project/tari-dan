// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! Length-aware metering for the bulk memory and table operators.
//!
//! The static cost function in [`super::metering`] prices one operator at one constant, which is
//! the wrong shape for an operator whose work is a runtime operand: `memory.copy` moves between 0
//! and 4 GiB of bytes for the same instruction. This middleware adds the length-proportional half
//! of the charge, emitted inline against the same remaining-points global the
//! [`tari_wasmer_middlewares::Metering`] middleware installs, so an oversized copy traps on the
//! meter before the copy runs rather than after.
//!
//! It must be pushed *after* `Metering`. Middleware stages run in push order, each consuming the
//! previous stage's output, so pushing second means the metering global indexes are already
//! recorded and the operators emitted here bypass static costing.

use std::sync::{Arc, Mutex};

use tari_wasmer_middlewares::Metering;
use wasmer::{
    GlobalInit,
    GlobalType,
    LocalFunctionIndex,
    ModuleInfo,
    Mutability,
    Type,
    sys::{FunctionMiddleware, MiddlewareError, MiddlewareReaderState, ModuleMiddleware},
    wasmparser::{BlockType, Operator},
};

use super::metering::CostFunction;

/// Points charged per byte moved by `memory.copy`, `memory.fill` and `memory.init`.
///
/// Metering points are calibrated at ~8.4M points/ms of validator CPU (see
/// [`tari_engine_types::limits::MAX_WASM_POINTS_PER_TRANSACTION`]). A host `memcpy` sustains on the
/// order of 10 GB/s, i.e. ~10M bytes/ms, so cost-neutral pricing is ~0.84 points/byte. Rounded up:
/// a bulk copy is never cheaper per byte than the meter believes.
const POINTS_PER_MEMORY_BYTE: i64 = 1;

/// Points charged per element touched by `table.copy`, `table.fill` and `table.grow`.
///
/// A table element is a host-side function reference, 8 to 16 bytes, and writing one costs more
/// than a byte of `memcpy` because each write goes through the reference representation rather than
/// a vectorised block move. Priced at the per-byte rate times the widest element.
const POINTS_PER_TABLE_ELEMENT: i64 = 16;

/// Points charged per 64 KiB page added by `memory.grow`.
///
/// A grow allocates and zeroes the new pages, which is the same per-byte work as `memory.fill`.
const POINTS_PER_MEMORY_PAGE: i64 = 65_536 * POINTS_PER_MEMORY_BYTE;

/// Indexes of the two globals this middleware appends to every module.
///
/// A middleware cannot add function locals — `locals_info` is read-only — so the operand a charge
/// is computed from is parked in a module global between being popped off the stack and being
/// pushed back. Threads are disabled in the engine's feature set, so a module global is
/// single-writer and this is sound.
#[derive(Debug, Clone, Copy)]
struct ScratchGlobals {
    /// `i32` holding the length operand of the operator being charged.
    len: u32,
    /// `i64` holding the points that length comes to.
    cost: u32,
}

pub struct BulkMetering {
    metering: Arc<Metering<CostFunction>>,
    scratch: Mutex<Option<ScratchGlobals>>,
}

impl BulkMetering {
    /// `metering` must be the same middleware instance that is pushed before this one; its global
    /// indexes are read when function middlewares are generated.
    pub fn new(metering: Arc<Metering<CostFunction>>) -> Self {
        Self {
            metering,
            scratch: Mutex::new(None),
        }
    }
}

impl std::fmt::Debug for BulkMetering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BulkMetering").finish_non_exhaustive()
    }
}

impl ModuleMiddleware for BulkMetering {
    fn generate_function_middleware<'a>(&self, _: LocalFunctionIndex) -> Box<dyn FunctionMiddleware<'a> + 'a> {
        let indexes = self
            .metering
            .global_indexes()
            .expect("Metering::transform_module_info must run before BulkMetering generates a function middleware");
        let scratch = self
            .scratch
            .lock()
            .expect("BulkMetering scratch lock")
            .expect("BulkMetering::transform_module_info must run before it generates a function middleware");

        Box::new(FunctionBulkMetering {
            remaining_points: indexes.remaining_points().as_u32(),
            points_exhausted: indexes.points_exhausted().as_u32(),
            scratch,
        })
    }

    fn transform_module_info(&self, module_info: &mut ModuleInfo) -> Result<(), MiddlewareError> {
        let mut scratch = self.scratch.lock().expect("BulkMetering scratch lock");
        assert!(
            scratch.is_none(),
            "BulkMetering::transform_module_info: a middleware instance serves exactly one module"
        );

        let len = module_info.globals.push(GlobalType::new(Type::I32, Mutability::Var));
        module_info.global_initializers.push(GlobalInit::I32Const(0));

        let cost = module_info.globals.push(GlobalType::new(Type::I64, Mutability::Var));
        module_info.global_initializers.push(GlobalInit::I64Const(0));

        *scratch = Some(ScratchGlobals {
            len: len.as_u32(),
            cost: cost.as_u32(),
        });

        Ok(())
    }
}

#[derive(Debug)]
struct FunctionBulkMetering {
    remaining_points: u32,
    points_exhausted: u32,
    scratch: ScratchGlobals,
}

/// Points charged per unit of the operand sitting on top of the stack, or `None` for an operator
/// whose cost the static table already prices in full.
fn rate_for(operator: &Operator) -> Option<i64> {
    match operator {
        // `[dst, src, len]` / `[dst, value, len]` / `[dst, offset, len]`: bytes.
        Operator::MemoryCopy { .. } | Operator::MemoryFill { .. } | Operator::MemoryInit { .. } => {
            Some(POINTS_PER_MEMORY_BYTE)
        },
        // `[dst, src, len]` / `[dst, value, len]` / `[value, delta]`: table elements.
        Operator::TableCopy { .. } |
        Operator::TableFill { .. } |
        Operator::TableInit { .. } |
        Operator::TableGrow { .. } => Some(POINTS_PER_TABLE_ELEMENT),
        // `[delta]`: 64 KiB pages.
        Operator::MemoryGrow { .. } => Some(POINTS_PER_MEMORY_PAGE),
        _ => None,
    }
}

impl<'a> FunctionMiddleware<'a> for FunctionBulkMetering {
    fn feed(&mut self, operator: Operator<'a>, state: &mut MiddlewareReaderState<'a>) -> Result<(), MiddlewareError> {
        let Some(rate) = rate_for(&operator) else {
            state.push_operator(operator);
            return Ok(());
        };

        // Every operator priced here leaves its count — bytes, elements or pages — on top of the
        // stack, so the charge is computed from the value popped here and the value is pushed back
        // unchanged before the operator runs.
        state.extend(&[
            Operator::GlobalSet {
                global_index: self.scratch.len,
            },
            Operator::GlobalGet {
                global_index: self.scratch.len,
            },
            Operator::I64ExtendI32U,
            Operator::I64Const { value: rate },
            Operator::I64Mul,
            Operator::GlobalSet {
                global_index: self.scratch.cost,
            },
            // if unsigned(remaining_points) < unsigned(cost) { points_exhausted = 1; trap }
            Operator::GlobalGet {
                global_index: self.remaining_points,
            },
            Operator::GlobalGet {
                global_index: self.scratch.cost,
            },
            Operator::I64LtU,
            Operator::If {
                blockty: BlockType::Empty,
            },
            Operator::I32Const { value: 1 },
            Operator::GlobalSet {
                global_index: self.points_exhausted,
            },
            Operator::Unreachable,
            Operator::End,
            // remaining_points -= cost
            Operator::GlobalGet {
                global_index: self.remaining_points,
            },
            Operator::GlobalGet {
                global_index: self.scratch.cost,
            },
            Operator::I64Sub,
            Operator::GlobalSet {
                global_index: self.remaining_points,
            },
            Operator::GlobalGet {
                global_index: self.scratch.len,
            },
        ]);

        state.push_operator(operator);

        Ok(())
    }
}
