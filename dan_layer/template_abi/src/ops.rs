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

use crate::rust::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum EngineOp {
    EmitLog = 0x00,
    ComponentInvoke = 0x01,
    ResourceInvoke = 0x02,
    VaultInvoke = 0x03,
    BucketInvoke = 0x04,
    WorkspaceInvoke = 0x05,
    GenerateUniqueId = 0x06,
}

impl EngineOp {
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0x00 => Some(EngineOp::EmitLog),
            0x01 => Some(EngineOp::ComponentInvoke),
            0x02 => Some(EngineOp::ResourceInvoke),
            0x03 => Some(EngineOp::VaultInvoke),
            0x04 => Some(EngineOp::BucketInvoke),
            0x05 => Some(EngineOp::WorkspaceInvoke),
            0x06 => Some(EngineOp::GenerateUniqueId),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> i32 {
        *self as i32
    }
}

impl fmt::Display for EngineOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
