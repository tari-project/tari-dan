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

use wasmer::{wasmparser::Operator, ModuleMiddleware};
use wasmer_middlewares::Metering;

pub fn middleware(limit: u64) -> impl ModuleMiddleware {
    Metering::new(limit, cost_function)
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
        Operator::I32Eqz
        | Operator::I32Eq
        | Operator::I32Ne
        | Operator::I32LtS
        | Operator::I32LtU
        | Operator::I32GtS
        | Operator::I32GtU
        | Operator::I32LeS
        | Operator::I32LeU
        | Operator::I32GeS
        | Operator::I32GeU
        | Operator::I64Eqz
        | Operator::I64Eq
        | Operator::I64Ne
        | Operator::I64LtS
        | Operator::I64LtU
        | Operator::I64GtS
        | Operator::I64GtU
        | Operator::I64LeS
        | Operator::I64LeU
        | Operator::I64GeS
        | Operator::I64GeU => 1,
        Operator::F32Eq
        | Operator::F32Ne
        | Operator::F32Lt
        | Operator::F32Gt
        | Operator::F32Le
        | Operator::F32Ge
        | Operator::F64Eq
        | Operator::F64Ne
        | Operator::F64Lt
        | Operator::F64Gt
        | Operator::F64Le
        | Operator::F64Ge => 4,
        Operator::I32Clz
        | Operator::I32Ctz
        | Operator::I32Popcnt
        | Operator::I32Sub
        | Operator::I32Mul
        | Operator::I32DivS
        | Operator::I32DivU
        | Operator::I32RemS
        | Operator::I32RemU
        | Operator::I32And
        | Operator::I32Or
        | Operator::I32Xor
        | Operator::I32Shl
        | Operator::I32ShrS
        | Operator::I32ShrU
        | Operator::I32Rotl
        | Operator::I32Rotr
        | Operator::I64Clz
        | Operator::I64Ctz
        | Operator::I64Popcnt
        | Operator::I64Add
        | Operator::I64Sub
        | Operator::I64Mul
        | Operator::I64DivS
        | Operator::I64DivU
        | Operator::I64RemS
        | Operator::I64RemU
        | Operator::I64And
        | Operator::I64Or
        | Operator::I64Xor
        | Operator::I64Shl
        | Operator::I64ShrS
        | Operator::I64ShrU
        | Operator::I64Rotl
        | Operator::I64Rotr => 1,
        Operator::F32Abs
        | Operator::F32Neg
        | Operator::F32Ceil
        | Operator::F32Floor
        | Operator::F32Trunc
        | Operator::F32Nearest => 4,
        Operator::F32Sqrt => 10,
        Operator::F32Add
        | Operator::F32Sub
        | Operator::F32Mul
        | Operator::F32Div
        | Operator::F32Min
        | Operator::F32Max
        | Operator::F32Copysign
        | Operator::F64Abs
        | Operator::F64Neg
        | Operator::F64Ceil
        | Operator::F64Floor
        | Operator::F64Trunc
        | Operator::F64Nearest => 4,
        Operator::F64Sqrt => 10,
        Operator::F64Add
        | Operator::F64Sub
        | Operator::F64Mul
        | Operator::F64Div
        | Operator::F64Min
        | Operator::F64Max
        | Operator::F64Copysign => 4,
        Operator::I32WrapI64
        | Operator::I32TruncF32S
        | Operator::I32TruncF32U
        | Operator::I32TruncF64S
        | Operator::I32TruncF64U
        | Operator::I64ExtendI32S
        | Operator::I64ExtendI32U
        | Operator::I64TruncF32S
        | Operator::I64TruncF32U
        | Operator::I64TruncF64S
        | Operator::I64TruncF64U
        | Operator::F32ConvertI32S
        | Operator::F32ConvertI32U
        | Operator::F32ConvertI64S
        | Operator::F32ConvertI64U
        | Operator::F32DemoteF64
        | Operator::F64ConvertI32S
        | Operator::F64ConvertI32U
        | Operator::F64ConvertI64S
        | Operator::F64ConvertI64U
        | Operator::F64PromoteF32
        | Operator::I32ReinterpretF32
        | Operator::I64ReinterpretF64
        | Operator::F32ReinterpretI32
        | Operator::F64ReinterpretI64
        | Operator::I32Extend8S
        | Operator::I32Extend16S
        | Operator::I64Extend8S
        | Operator::I64Extend16S
        | Operator::I64Extend32S
        | Operator::I32TruncSatF32S
        | Operator::I32TruncSatF32U
        | Operator::I32TruncSatF64S
        | Operator::I32TruncSatF64U
        | Operator::I64TruncSatF32S
        | Operator::I64TruncSatF32U
        | Operator::I64TruncSatF64S
        | Operator::I64TruncSatF64U => 1,
        Operator::MemoryInit { .. } => 4,
        Operator::DataDrop { .. } => 1,
        Operator::MemoryCopy { .. } | Operator::MemoryFill { .. } => 2,
        Operator::TableInit { .. }
        | Operator::ElemDrop { .. }
        | Operator::TableCopy { .. }
        | Operator::TableFill { .. }
        | Operator::TableGet { .. }
        | Operator::TableSet { .. }
        | Operator::TableGrow { .. }
        | Operator::TableSize { .. } => 2,
        Operator::MemoryAtomicNotify { .. }
        | Operator::MemoryAtomicWait32 { .. }
        | Operator::MemoryAtomicWait64 { .. } => 3,
        Operator::AtomicFence { .. } => 2,

        Operator::Unreachable
        | Operator::Nop
        | Operator::Block { .. }
        | Operator::Loop { .. }
        | Operator::If { .. }
        | Operator::Else
        | Operator::Try { .. }
        | Operator::Catch { .. }
        | Operator::Throw { .. }
        | Operator::Rethrow { .. }
        | Operator::End
        | Operator::Br { .. }
        | Operator::BrIf { .. }
        | Operator::BrTable { .. }
        | Operator::Return
        | Operator::ReturnCall { .. }
        | Operator::ReturnCallIndirect { .. }
        | Operator::CatchAll
        | Operator::Drop
        | Operator::Select
        | Operator::TypedSelect { .. } => 0,
        _ => 1,
    }
}
