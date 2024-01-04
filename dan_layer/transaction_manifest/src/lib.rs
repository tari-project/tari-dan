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

use std::{collections::HashMap, str::FromStr};

use proc_macro2::TokenStream;
use syn::parse2;
use tari_engine_types::instruction::Instruction;

use self::ast::ManifestAst;
pub use crate::value::ManifestValue;
use crate::{error::ManifestError, generator::ManifestInstructionGenerator};

mod ast;
mod error;
mod generator;
mod parser;
mod value;

pub fn parse_manifest(
    input: &str,
    globals: HashMap<String, ManifestValue>,
) -> Result<ManifestInstructions, ManifestError> {
    let tokens = TokenStream::from_str(input).map_err(|e| ManifestError::LexError(e.to_string()))?;
    let ast = parse2::<ManifestAst>(tokens)?;

    ManifestInstructionGenerator::new(globals).generate_instructions(ast)
}

pub struct ManifestInstructions {
    pub instructions: Vec<Instruction>,
    pub fee_instructions: Vec<Instruction>,
}
