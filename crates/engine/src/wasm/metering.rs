//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_wasmer_middlewares::Metering;
use wasmer::wasmparser::Operator;

/// The static per-operator cost table, as a function pointer so that the concrete `Metering` type
/// can be named. [`super::bulk_metering::BulkMetering`] holds the same instance to read its global
/// indexes.
pub type CostFunction = fn(&Operator) -> u64;

pub fn middleware(limit: u64) -> Metering<CostFunction> {
    Metering::new(limit, cost_function as CostFunction)
}

#[allow(clippy::too_many_lines)]
fn cost_function(op: &Operator) -> u64 {
    match op {
        Operator::LocalGet { .. } | Operator::I32Const { .. } => 1,
        Operator::I32Add { .. } => 1,
        Operator::Call { .. } => 4,
        Operator::CallIndirect { .. } => 4,
        Operator::Delegate { .. } => 1,
        Operator::LocalSet { .. } => 1,
        Operator::LocalTee { .. } => 1,
        Operator::GlobalGet { .. } => 0,
        Operator::GlobalSet { .. } => 1,
        Operator::I32Load { .. } => 1,
        Operator::I64Load { .. } => 1,
        Operator::F32Load { .. } => 1,
        Operator::F64Load { .. } => 1,
        Operator::I32Load8S { .. } => 1,
        Operator::I32Load8U { .. } => 1,
        Operator::I32Load16S { .. } => 1,
        Operator::I32Load16U { .. } => 1,
        Operator::I64Load8S { .. } => 1,
        Operator::I64Load8U { .. } => 1,
        Operator::I64Load16S { .. } => 1,
        Operator::I64Load16U { .. } => 1,
        Operator::I64Load32S { .. } => 1,
        Operator::I64Load32U { .. } => 1,
        Operator::I32Store { .. } => 2,
        Operator::I64Store { .. } => 2,
        Operator::F32Store { .. } => 4,
        Operator::F64Store { .. } => 4,
        Operator::I32Store8 { .. } => 2,
        Operator::I32Store16 { .. } => 2,
        Operator::I64Store8 { .. } => 2,
        Operator::I64Store16 { .. } => 2,
        Operator::I64Store32 { .. } => 2,
        Operator::MemorySize { .. } => 1,
        Operator::MemoryGrow { .. } => 4,
        Operator::I64Const { .. } => 0,
        Operator::F32Const { .. } => 1,
        Operator::F64Const { .. } => 1,
        Operator::RefNull { .. } => 0,
        Operator::RefIsNull => 0,
        Operator::RefFunc { .. } => 1,
        Operator::I32Eqz |
        Operator::I32Eq |
        Operator::I32Ne |
        Operator::I32LtS |
        Operator::I32LtU |
        Operator::I32GtS |
        Operator::I32GtU |
        Operator::I32LeS |
        Operator::I32LeU |
        Operator::I32GeS |
        Operator::I32GeU |
        Operator::I64Eqz |
        Operator::I64Eq |
        Operator::I64Ne |
        Operator::I64LtS |
        Operator::I64LtU |
        Operator::I64GtS |
        Operator::I64GtU |
        Operator::I64LeS |
        Operator::I64LeU |
        Operator::I64GeS |
        Operator::I64GeU => 1,
        Operator::F32Eq |
        Operator::F32Ne |
        Operator::F32Lt |
        Operator::F32Gt |
        Operator::F32Le |
        Operator::F32Ge |
        Operator::F64Eq |
        Operator::F64Ne |
        Operator::F64Lt |
        Operator::F64Gt |
        Operator::F64Le |
        Operator::F64Ge => 4,
        // Integer multiply and divide are far more expensive to execute than an add, but were
        // historically metered identically (1 point). Re-costed from measured per-op latency (see
        // the `metering_recost` engine example) — division dominates a CPU-bound transaction's
        // wall-clock time, so under-pricing it let a transaction buy far more execution time than
        // its metered cost implied. Values target the slow path on a typical x86 validator (64-bit
        // divide ~30-90 cycles, 32-bit ~20-30, multiply ~3); re-measure on the slowest supported
        // hardware and raise if needed.
        Operator::I64DivU | Operator::I64DivS | Operator::I64RemU | Operator::I64RemS => 30,
        Operator::I32DivU | Operator::I32DivS | Operator::I32RemU | Operator::I32RemS => 20,
        Operator::I64Mul => 3,
        Operator::I32Mul => 2,
        Operator::I32Clz |
        Operator::I32Ctz |
        Operator::I32Popcnt |
        Operator::I32Sub |
        Operator::I32And |
        Operator::I32Or |
        Operator::I32Xor |
        Operator::I32Shl |
        Operator::I32ShrS |
        Operator::I32ShrU |
        Operator::I32Rotl |
        Operator::I32Rotr |
        Operator::I64Clz |
        Operator::I64Ctz |
        Operator::I64Popcnt |
        Operator::I64Add |
        Operator::I64Sub |
        Operator::I64And |
        Operator::I64Or |
        Operator::I64Xor |
        Operator::I64Shl |
        Operator::I64ShrS |
        Operator::I64ShrU |
        Operator::I64Rotl |
        Operator::I64Rotr => 1,
        Operator::F32Abs |
        Operator::F32Neg |
        Operator::F32Ceil |
        Operator::F32Floor |
        Operator::F32Trunc |
        Operator::F32Nearest => 4,
        Operator::F32Sqrt => 20,
        Operator::F32Div => 12,
        Operator::F32Add |
        Operator::F32Sub |
        Operator::F32Mul |
        Operator::F32Min |
        Operator::F32Max |
        Operator::F32Copysign |
        Operator::F64Abs |
        Operator::F64Neg |
        Operator::F64Ceil |
        Operator::F64Floor |
        Operator::F64Trunc |
        Operator::F64Nearest => 4,
        Operator::F64Sqrt => 20,
        Operator::F64Div => 15,
        Operator::F64Add |
        Operator::F64Sub |
        Operator::F64Mul |
        Operator::F64Min |
        Operator::F64Max |
        Operator::F64Copysign => 4,
        Operator::I32WrapI64 |
        Operator::I32TruncF32S |
        Operator::I32TruncF32U |
        Operator::I32TruncF64S |
        Operator::I32TruncF64U |
        Operator::I64ExtendI32S |
        Operator::I64ExtendI32U |
        Operator::I64TruncF32S |
        Operator::I64TruncF32U |
        Operator::I64TruncF64S |
        Operator::I64TruncF64U |
        Operator::F32ConvertI32S |
        Operator::F32ConvertI32U |
        Operator::F32ConvertI64S |
        Operator::F32ConvertI64U |
        Operator::F32DemoteF64 |
        Operator::F64ConvertI32S |
        Operator::F64ConvertI32U |
        Operator::F64ConvertI64S |
        Operator::F64ConvertI64U |
        Operator::F64PromoteF32 |
        Operator::I32ReinterpretF32 |
        Operator::I64ReinterpretF64 |
        Operator::F32ReinterpretI32 |
        Operator::F64ReinterpretI64 |
        Operator::I32Extend8S |
        Operator::I32Extend16S |
        Operator::I64Extend8S |
        Operator::I64Extend16S |
        Operator::I64Extend32S |
        Operator::I32TruncSatF32S |
        Operator::I32TruncSatF32U |
        Operator::I32TruncSatF64S |
        Operator::I32TruncSatF64U |
        Operator::I64TruncSatF32S |
        Operator::I64TruncSatF32U |
        Operator::I64TruncSatF64S |
        Operator::I64TruncSatF64U => 1,
        Operator::MemoryInit { .. } => 4,
        Operator::DataDrop { .. } => 1,
        Operator::MemoryCopy { .. } | Operator::MemoryFill { .. } => 2,
        Operator::TableInit { .. } |
        Operator::ElemDrop { .. } |
        Operator::TableCopy { .. } |
        Operator::TableFill { .. } |
        Operator::TableGet { .. } |
        Operator::TableSet { .. } |
        Operator::TableGrow { .. } |
        Operator::TableSize { .. } => 2,
        Operator::MemoryAtomicNotify { .. } |
        Operator::MemoryAtomicWait32 { .. } |
        Operator::MemoryAtomicWait64 { .. } => 3,
        Operator::AtomicFence { .. } => 2,

        // SIMD (V128) instructions: cost 4x scalar equivalents since they operate on 128-bit
        // vectors (4 lanes of 32-bit or 2 lanes of 64-bit values simultaneously).
        //
        // Memory: load/store cost matches scalar load(1)/store(2) scaled by 4x for wider access
        Operator::V128Load { .. } |
        Operator::V128Load8x8S { .. } |
        Operator::V128Load8x8U { .. } |
        Operator::V128Load16x4S { .. } |
        Operator::V128Load16x4U { .. } |
        Operator::V128Load32x2S { .. } |
        Operator::V128Load32x2U { .. } |
        Operator::V128Load8Splat { .. } |
        Operator::V128Load16Splat { .. } |
        Operator::V128Load32Splat { .. } |
        Operator::V128Load64Splat { .. } |
        Operator::V128Load32Zero { .. } |
        Operator::V128Load64Zero { .. } |
        Operator::V128Load8Lane { .. } |
        Operator::V128Load16Lane { .. } |
        Operator::V128Load32Lane { .. } |
        Operator::V128Load64Lane { .. } => 4,
        Operator::V128Store { .. } |
        Operator::V128Store8Lane { .. } |
        Operator::V128Store16Lane { .. } |
        Operator::V128Store32Lane { .. } |
        Operator::V128Store64Lane { .. } => 8,
        // V128 const and bitwise: cheap lane-parallel ops
        Operator::V128Const { .. } => 1,
        Operator::V128Not |
        Operator::V128And |
        Operator::V128AndNot |
        Operator::V128Or |
        Operator::V128Xor |
        Operator::V128Bitselect |
        Operator::V128AnyTrue => 2,
        // Integer SIMD arithmetic (4x scalar cost)
        Operator::I8x16Splat |
        Operator::I16x8Splat |
        Operator::I32x4Splat |
        Operator::I64x2Splat |
        Operator::I8x16ExtractLaneS { .. } |
        Operator::I8x16ExtractLaneU { .. } |
        Operator::I8x16ReplaceLane { .. } |
        Operator::I16x8ExtractLaneS { .. } |
        Operator::I16x8ExtractLaneU { .. } |
        Operator::I16x8ReplaceLane { .. } |
        Operator::I32x4ExtractLane { .. } |
        Operator::I32x4ReplaceLane { .. } |
        Operator::I64x2ExtractLane { .. } |
        Operator::I64x2ReplaceLane { .. } => 2,
        Operator::I8x16Eq |
        Operator::I8x16Ne |
        Operator::I8x16LtS |
        Operator::I8x16LtU |
        Operator::I8x16GtS |
        Operator::I8x16GtU |
        Operator::I8x16LeS |
        Operator::I8x16LeU |
        Operator::I8x16GeS |
        Operator::I8x16GeU |
        Operator::I16x8Eq |
        Operator::I16x8Ne |
        Operator::I16x8LtS |
        Operator::I16x8LtU |
        Operator::I16x8GtS |
        Operator::I16x8GtU |
        Operator::I16x8LeS |
        Operator::I16x8LeU |
        Operator::I16x8GeS |
        Operator::I16x8GeU |
        Operator::I32x4Eq |
        Operator::I32x4Ne |
        Operator::I32x4LtS |
        Operator::I32x4LtU |
        Operator::I32x4GtS |
        Operator::I32x4GtU |
        Operator::I32x4LeS |
        Operator::I32x4LeU |
        Operator::I32x4GeS |
        Operator::I32x4GeU |
        Operator::I64x2Eq |
        Operator::I64x2Ne |
        Operator::I64x2LtS |
        Operator::I64x2GtS |
        Operator::I64x2LeS |
        Operator::I64x2GeS => 4,
        Operator::I8x16Abs |
        Operator::I8x16Neg |
        Operator::I8x16AllTrue |
        Operator::I8x16Bitmask |
        Operator::I8x16Popcnt |
        Operator::I8x16Add |
        Operator::I8x16AddSatS |
        Operator::I8x16AddSatU |
        Operator::I8x16Sub |
        Operator::I8x16SubSatS |
        Operator::I8x16SubSatU |
        Operator::I8x16MinS |
        Operator::I8x16MinU |
        Operator::I8x16MaxS |
        Operator::I8x16MaxU |
        Operator::I8x16AvgrU |
        Operator::I8x16Shl |
        Operator::I8x16ShrS |
        Operator::I8x16ShrU |
        Operator::I8x16NarrowI16x8S |
        Operator::I8x16NarrowI16x8U |
        Operator::I8x16Swizzle |
        Operator::I8x16Shuffle { .. } |
        Operator::I16x8Abs |
        Operator::I16x8Neg |
        Operator::I16x8AllTrue |
        Operator::I16x8Bitmask |
        Operator::I16x8Add |
        Operator::I16x8AddSatS |
        Operator::I16x8AddSatU |
        Operator::I16x8Sub |
        Operator::I16x8SubSatS |
        Operator::I16x8SubSatU |
        Operator::I16x8Mul |
        Operator::I16x8MinS |
        Operator::I16x8MinU |
        Operator::I16x8MaxS |
        Operator::I16x8MaxU |
        Operator::I16x8AvgrU |
        Operator::I16x8Shl |
        Operator::I16x8ShrS |
        Operator::I16x8ShrU |
        Operator::I16x8NarrowI32x4S |
        Operator::I16x8NarrowI32x4U |
        Operator::I16x8Q15MulrSatS |
        Operator::I16x8ExtAddPairwiseI8x16S |
        Operator::I16x8ExtAddPairwiseI8x16U |
        Operator::I16x8ExtendHighI8x16S |
        Operator::I16x8ExtendHighI8x16U |
        Operator::I16x8ExtendLowI8x16S |
        Operator::I16x8ExtendLowI8x16U |
        Operator::I16x8ExtMulHighI8x16S |
        Operator::I16x8ExtMulHighI8x16U |
        Operator::I16x8ExtMulLowI8x16S |
        Operator::I16x8ExtMulLowI8x16U |
        Operator::I32x4Abs |
        Operator::I32x4Neg |
        Operator::I32x4AllTrue |
        Operator::I32x4Bitmask |
        Operator::I32x4Add |
        Operator::I32x4Sub |
        Operator::I32x4Mul |
        Operator::I32x4MinS |
        Operator::I32x4MinU |
        Operator::I32x4MaxS |
        Operator::I32x4MaxU |
        Operator::I32x4Shl |
        Operator::I32x4ShrS |
        Operator::I32x4ShrU |
        Operator::I32x4DotI16x8S |
        Operator::I32x4ExtAddPairwiseI16x8S |
        Operator::I32x4ExtAddPairwiseI16x8U |
        Operator::I32x4ExtendHighI16x8S |
        Operator::I32x4ExtendHighI16x8U |
        Operator::I32x4ExtendLowI16x8S |
        Operator::I32x4ExtendLowI16x8U |
        Operator::I32x4ExtMulHighI16x8S |
        Operator::I32x4ExtMulHighI16x8U |
        Operator::I32x4ExtMulLowI16x8S |
        Operator::I32x4ExtMulLowI16x8U |
        Operator::I64x2Abs |
        Operator::I64x2Neg |
        Operator::I64x2AllTrue |
        Operator::I64x2Bitmask |
        Operator::I64x2Add |
        Operator::I64x2Sub |
        Operator::I64x2Mul |
        Operator::I64x2Shl |
        Operator::I64x2ShrS |
        Operator::I64x2ShrU |
        Operator::I64x2ExtendHighI32x4S |
        Operator::I64x2ExtendHighI32x4U |
        Operator::I64x2ExtendLowI32x4S |
        Operator::I64x2ExtendLowI32x4U |
        Operator::I64x2ExtMulHighI32x4S |
        Operator::I64x2ExtMulHighI32x4U |
        Operator::I64x2ExtMulLowI32x4S |
        Operator::I64x2ExtMulLowI32x4U => 4,
        // Conversion between integer SIMD types
        Operator::I32x4TruncSatF32x4S |
        Operator::I32x4TruncSatF32x4U |
        Operator::I32x4TruncSatF64x2SZero |
        Operator::I32x4TruncSatF64x2UZero => 4,
        // Float SIMD (4x scalar float cost)
        Operator::F32x4Splat |
        Operator::F32x4ExtractLane { .. } |
        Operator::F32x4ReplaceLane { .. } |
        Operator::F64x2Splat |
        Operator::F64x2ExtractLane { .. } |
        Operator::F64x2ReplaceLane { .. } => 4,
        Operator::F32x4Eq |
        Operator::F32x4Ne |
        Operator::F32x4Lt |
        Operator::F32x4Gt |
        Operator::F32x4Le |
        Operator::F32x4Ge |
        Operator::F64x2Eq |
        Operator::F64x2Ne |
        Operator::F64x2Lt |
        Operator::F64x2Gt |
        Operator::F64x2Le |
        Operator::F64x2Ge => 16,
        Operator::F32x4Abs |
        Operator::F32x4Neg |
        Operator::F32x4Ceil |
        Operator::F32x4Floor |
        Operator::F32x4Trunc |
        Operator::F32x4Nearest |
        Operator::F32x4Add |
        Operator::F32x4Sub |
        Operator::F32x4Mul |
        Operator::F32x4Div |
        Operator::F32x4Min |
        Operator::F32x4Max |
        Operator::F32x4PMin |
        Operator::F32x4PMax |
        Operator::F64x2Abs |
        Operator::F64x2Neg |
        Operator::F64x2Ceil |
        Operator::F64x2Floor |
        Operator::F64x2Trunc |
        Operator::F64x2Nearest |
        Operator::F64x2Add |
        Operator::F64x2Sub |
        Operator::F64x2Mul |
        Operator::F64x2Div |
        Operator::F64x2Min |
        Operator::F64x2Max |
        Operator::F64x2PMin |
        Operator::F64x2PMax |
        Operator::F32x4ConvertI32x4S |
        Operator::F32x4ConvertI32x4U |
        Operator::F32x4DemoteF64x2Zero |
        Operator::F64x2ConvertLowI32x4S |
        Operator::F64x2ConvertLowI32x4U |
        Operator::F64x2PromoteLowF32x4 => 16,
        Operator::F32x4Sqrt | Operator::F64x2Sqrt => 40,
        // Relaxed SIMD instructions
        Operator::I8x16RelaxedSwizzle |
        Operator::I8x16RelaxedLaneselect |
        Operator::I16x8RelaxedLaneselect |
        Operator::I16x8RelaxedQ15mulrS |
        Operator::I16x8RelaxedDotI8x16I7x16S |
        Operator::I32x4RelaxedLaneselect |
        Operator::I32x4RelaxedTruncF32x4S |
        Operator::I32x4RelaxedTruncF32x4U |
        Operator::I32x4RelaxedTruncF64x2SZero |
        Operator::I32x4RelaxedTruncF64x2UZero |
        Operator::I32x4RelaxedDotI8x16I7x16AddS |
        Operator::I64x2RelaxedLaneselect |
        Operator::F32x4RelaxedMadd |
        Operator::F32x4RelaxedNmadd |
        Operator::F32x4RelaxedMin |
        Operator::F32x4RelaxedMax |
        Operator::F64x2RelaxedMadd |
        Operator::F64x2RelaxedNmadd |
        Operator::F64x2RelaxedMin |
        Operator::F64x2RelaxedMax => 8,

        Operator::Unreachable |
        Operator::Nop |
        Operator::Block { .. } |
        Operator::Loop { .. } |
        Operator::If { .. } |
        Operator::Else |
        Operator::Try { .. } |
        Operator::Catch { .. } |
        Operator::Throw { .. } |
        Operator::Rethrow { .. } |
        Operator::End |
        Operator::Br { .. } |
        Operator::BrIf { .. } |
        Operator::BrTable { .. } |
        Operator::Return |
        Operator::ReturnCall { .. } |
        Operator::ReturnCallIndirect { .. } |
        Operator::CatchAll |
        Operator::Drop |
        Operator::Select |
        Operator::TypedSelect { .. } => 1,
        // An operator this table does not name. The engine's `Features` set admits only the
        // proposals the cases above cover, so this arm is reached when a wasmer upgrade introduces
        // an operator before it is priced here. The fallback sits far above every named cost, so
        // such an operator is costly to execute until someone prices it.
        _ => 1_000,
    }
}
