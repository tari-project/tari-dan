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

use tari_template_abi::TemplateDef;

use crate::{flow::FlowInstance, wasm::LoadedWasmTemplate};

#[derive(Debug, Clone)]
pub enum LoadedTemplate {
    Wasm(LoadedWasmTemplate),
    Flow(FlowInstance),
}

impl LoadedTemplate {
    /// Returns a "friendly name" for the template typically provided by the author. This name is not guaranteed to be
    /// unique.
    pub fn template_name(&self) -> &str {
        match self {
            LoadedTemplate::Wasm(wasm) => wasm.template_name(),
            LoadedTemplate::Flow(flow) => flow.name(),
        }
    }

    pub fn template_def(&self) -> &TemplateDef {
        match self {
            LoadedTemplate::Wasm(wasm) => wasm.template_def(),
            LoadedTemplate::Flow(flow) => {
                todo!()
            },
        }
    }

    pub fn code_size(&self) -> usize {
        match self {
            LoadedTemplate::Wasm(wasm) => wasm.code_size(),
            LoadedTemplate::Flow(_) => {
                todo!()
            },
        }
    }
}

impl From<LoadedWasmTemplate> for LoadedTemplate {
    fn from(module: LoadedWasmTemplate) -> Self {
        Self::Wasm(module)
    }
}
