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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instruction {
    CallFunction {
        package_address: String,
        template: String,
        function: String,
        proofs: Vec<String>, // variables with the badge_proofs
        args: Vec<Arg>,
        return_variables: Vec<VariableIdent>,
    },
    CallMethod {
        package_address: String,
        component_address: String,
        method: String,
        proofs: Vec<String>, // variables with the badge_proofs
        args: Vec<Arg>,
        return_variables: Vec<VariableIdent>,
    },
    BucketSplit {
        input: VariableIdent,
        amount: u32,                  // TODO: use an amount type
        output_main: VariableIdent,   // name of the new variable with the specified amount
        output_change: VariableIdent, // name of the new variable that will hold the change
    },
    BucketJoin {
        inputs: Vec<VariableIdent>, // names of all the bucket variables to join
        output: VariableIdent,      // name of the new output variable
    },
    GenerateBadgeProof {
        input: VariableIdent, // must be a bucket variable
        output: VariableIdent,
    },
    AssertEq {
        input_a: Arg,
        input_b: Arg,
    },
    AssertNe {
        input_a: Arg,
        input_b: Arg,
    },
    AssertGt {
        input_a: Arg,
        input_b: Arg,
    },
    AssertLt {
        input_a: Arg,
        input_b: Arg,
    },
}

pub type VariableIdent = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Literal(Value),
    Variable(VariableIdent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    // Basic values
    Unit,
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(i128),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    String(String),

    // Complex values
    Amount(u64),
    Tuple(Vec<Value>),
    ComponentAddress(String), // TODO: address type
    Bucket(String),           // TODO: resource address
    Proof(String),            // TODO: resource addess
}
