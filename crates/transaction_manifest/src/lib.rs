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

use proc_macro2::{TokenStream, TokenTree};
use syn::parse2;
use tari_ootle_transaction::{Blob, Blobs, Instruction};
use tari_template_lib_types::TemplateAddress;

use self::ast::ManifestAst;
pub use crate::value::ManifestValue;
use crate::{error::ManifestError, generator::ManifestInstructionGenerator, value::MAX_LIST_DEPTH};

mod ast;
mod cbor_literal;
mod error;
mod generator;
mod parser;
mod value;

/// Parse a manifest into a set of instructions plus the ordered `Blobs` they reference.
///
/// `globals` maps variable names to typed manifest values (e.g. component addresses).
/// `templates` maps imported template aliases to their on-chain addresses.
/// `blob_inputs` maps `blob!(name)` references to their byte payloads. The returned
/// `ManifestInstructions.blobs` lists those blobs in order of first reference and is what the
/// caller should attach to the transaction.
pub fn parse_manifest(
    input: &str,
    globals: HashMap<String, ManifestValue>,
    templates: HashMap<String, TemplateAddress>,
    blob_inputs: HashMap<String, Blob>,
) -> Result<ManifestInstructions, ManifestError> {
    // A wallet daemon parses manifests submitted by its clients, so every input must end in an error rather than
    // exhaust the stack. Lexing is iterative; `syn` recurses once per group level, and every recursion in the manifest
    // parser descends into a nested group, so capping group depth here bounds the stack the parse needs.
    if input.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::ManifestTooLarge {
            size: input.len(),
            max: MAX_MANIFEST_BYTES,
        });
    }

    let tokens = TokenStream::from_str(input).map_err(|e| ManifestError::LexError(e.to_string()))?;
    check_group_depth(&tokens)?;
    let ast = parse2::<ManifestAst>(tokens)?;
    ManifestInstructionGenerator::new(globals, templates, blob_inputs).generate_instructions(ast)
}

/// Sanity bound on manifest source size. Generous — real manifests are a few KiB at most.
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// A valid manifest nests groups no deeper than a function body, a call and a macro around a value, and a value
/// nests at most `MAX_LIST_DEPTH` levels. Twice that leaves the list and CBOR depth checks to reject a value a few
/// levels too deep with their own errors.
const MAX_GROUP_DEPTH: usize = 2 * MAX_LIST_DEPTH;

fn check_group_depth(tokens: &TokenStream) -> Result<(), ManifestError> {
    let mut stack = vec![tokens.clone().into_iter()];
    while let Some(level) = stack.last_mut() {
        match level.next() {
            Some(TokenTree::Group(group)) => {
                if stack.len() > MAX_GROUP_DEPTH {
                    return Err(ManifestError::NestingTooDeep { max: MAX_GROUP_DEPTH });
                }
                stack.push(group.stream().into_iter());
            },
            Some(_) => {},
            None => {
                stack.pop();
            },
        }
    }
    Ok(())
}

pub struct ManifestInstructions {
    pub instructions: Vec<Instruction>,
    pub fee_instructions: Vec<Instruction>,
    /// Blobs referenced by `blob!(name)` in the manifest, in the order they were first
    /// encountered. Indices in `InstructionArg::Blob(idx)` map directly into this list.
    pub blobs: Blobs,
}

#[cfg(test)]
mod size_cap_tests {
    use super::*;

    #[test]
    fn rejects_oversized_source() {
        let src = "a".repeat(MAX_MANIFEST_BYTES + 1);
        let res = parse_manifest(&src, HashMap::new(), Default::default(), Default::default());
        assert!(matches!(res, Err(ManifestError::ManifestTooLarge { size, max })
            if size == MAX_MANIFEST_BYTES + 1 && max == MAX_MANIFEST_BYTES));
    }

    #[test]
    fn accepts_normal_manifest() {
        // A normal manifest parses past the size cap and lex/parse (it fails later only because
        // `foo` is undefined, not at the cap).
        let src = "fn main() { let a = foo(bar([1, 2, 3])); }";
        let res = parse_manifest(src, HashMap::new(), Default::default(), Default::default());
        assert!(!matches!(res, Err(ManifestError::ManifestTooLarge { .. })));
    }
}
