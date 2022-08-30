// TODO: remove this
#![allow(dead_code)]

pub struct TransactionBuilder {
    instructions: Vec<Instruction>,
}

impl TransactionBuilder {
    pub fn new() -> Self {
        // TODO: pass the runtime as an arg
        Self { instructions: vec![] }
    }

    pub fn add_instruction(&mut self, instruction: Instruction) {
        self.instructions.push(instruction);
    }

    // TODO: add "build" method (serialize, sign, etc)
    // TODO: add "run" method (to run in an engine)
}

#[derive(Debug, Clone)]
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

type VariableIdent = String;

#[derive(Debug, Clone)]
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
    Tuple(Vec<Value>),
    ComponentAddress(String), // TODO: address type
    Bucket(String),           // TODO: resource address
    Proof(String),            // TODO: resource addess
}
