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

use std::io;

use serde::{Deserialize, Serialize};
use tari_bor::Decode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub raw: Vec<u8>,
    pub return_type: Type,
}

impl ExecutionResult {
    pub fn empty() -> Self {
        ExecutionResult {
            raw: Vec::new(),
            return_type: Type::Unit,
        }
    }

    pub fn decode<T: Decode>(&self) -> io::Result<T> {
        tari_bor::decode(&self.raw)
    }
}

// TODO: This is to avoid adding serde to the abi crate - that probably isn't so bad if we use feature flags, but I'm
//       still cautious
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Type {
    Unit,
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    String,
    Other { name: String },
}

impl From<tari_template_abi::Type> for Type {
    fn from(val: tari_template_abi::Type) -> Self {
        match val {
            tari_template_abi::Type::Unit => Type::Unit,
            tari_template_abi::Type::Bool => Type::Bool,
            tari_template_abi::Type::I8 => Type::I8,
            tari_template_abi::Type::I16 => Type::I16,
            tari_template_abi::Type::I32 => Type::I32,
            tari_template_abi::Type::I64 => Type::I64,
            tari_template_abi::Type::I128 => Type::I128,
            tari_template_abi::Type::U8 => Type::U8,
            tari_template_abi::Type::U16 => Type::U16,
            tari_template_abi::Type::U32 => Type::U32,
            tari_template_abi::Type::U64 => Type::U64,
            tari_template_abi::Type::U128 => Type::U128,
            tari_template_abi::Type::String => Type::String,
            tari_template_abi::Type::Other { name } => Type::Other { name },
        }
    }
}
