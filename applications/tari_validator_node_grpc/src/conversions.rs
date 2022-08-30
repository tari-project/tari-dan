//  Copyright 2022, The Tari Project
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

use std::{
    borrow::Borrow,
    convert::{TryFrom, TryInto},
};

use borsh::de::BorshDeserialize;
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_dan_engine::instruction::{Instruction, Transaction};
use tari_template_lib::{args::Arg, Hash};
use tari_utilities::ByteArray;

use crate::rpc::{self as grpc, SubmitTransactionRequest};

impl TryFrom<grpc::Signature> for Signature {
    type Error = String;

    fn try_from(sig: grpc::Signature) -> Result<Self, Self::Error> {
        let public_nonce =
            PublicKey::from_bytes(&sig.public_nonce).map_err(|_| "Could not get public nonce".to_string())?;
        let signature = PrivateKey::from_bytes(&sig.signature).map_err(|_| "Could not get signature".to_string())?;

        Ok(Self::new(public_nonce, signature))
    }
}

impl<T: Borrow<Signature>> From<T> for grpc::Signature {
    fn from(sig: T) -> Self {
        Self {
            public_nonce: sig.borrow().get_public_nonce().to_vec(),
            signature: sig.borrow().get_signature().to_vec(),
        }
    }
}

impl TryFrom<grpc::SubmitTransactionRequest> for Transaction {
    type Error = String;

    fn try_from(request: grpc::SubmitTransactionRequest) -> Result<Self, Self::Error> {
        let instructions = request
            .instructions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<Instruction>, _>>()?;
        let signature: Signature = request.signature.ok_or("invalid signature")?.try_into()?;
        let instruction_signature = signature.try_into()?;
        let sender_public_key =
            PublicKey::from_bytes(&request.sender_public_key).map_err(|_| "invalid sender_public_key")?;
        let transaction = Transaction {
            instructions,
            signature: instruction_signature,
            sender_public_key,
        };

        Ok(transaction)
    }
}

impl TryFrom<grpc::Instruction> for Instruction {
    type Error = String;

    fn try_from(request: grpc::Instruction) -> Result<Self, Self::Error> {
        let package_address =
            Hash::deserialize(&mut &request.package_address[..]).map_err(|_| "invalid package_addresss")?;
        let args = request
            .args
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<Arg>, _>>()?;
        let instruction = match request.instruction_type {
            // function
            0 => {
                let template = request.template;
                let function = request.function;
                Instruction::CallFunction {
                    template,
                    function,
                    args,
                    package_address,
                }
            },
            // method
            1 => {
                let component_address =
                    Hash::deserialize(&mut &request.component_address[..]).map_err(|_| "invalid component_address")?;
                let method = request.method;
                Instruction::CallMethod {
                    method,
                    args,
                    package_address,
                    component_address,
                }
            },
            2 => Instruction::PutLastInstructionOutputOnWorkspace { key: request.key },
            _ => return Err("invalid instruction_type".to_string()),
        };

        Ok(instruction)
    }
}

impl From<Transaction> for SubmitTransactionRequest {
    fn from(transaction: Transaction) -> Self {
        let instructions = transaction.instructions.into_iter().map(Into::into).collect();
        let signature = transaction.signature.signature();
        let sender_public_key = transaction.sender_public_key.to_vec();

        SubmitTransactionRequest {
            instructions,
            signature: Some(signature.into()),
            sender_public_key,
        }
    }
}

impl From<Instruction> for grpc::Instruction {
    fn from(instruction: Instruction) -> Self {
        let mut result = grpc::Instruction::default();

        match instruction {
            Instruction::CallFunction {
                template,
                function,
                args,
                package_address,
            } => {
                result.instruction_type = 0;
                result.package_address = package_address.to_vec();
                result.template = template;
                result.function = function;
                result.args = args.into_iter().map(Into::into).collect();
            },
            Instruction::CallMethod {
                method,
                args,
                package_address,
                component_address,
            } => {
                result.instruction_type = 1;
                result.package_address = package_address.to_vec();
                result.component_address = component_address.to_vec();
                result.method = method;
                result.args = args.into_iter().map(Into::into).collect();
            },
            Instruction::PutLastInstructionOutputOnWorkspace { key } => {
                result.instruction_type = 2;
                result.key = key;
            },
        }

        result
    }
}

impl TryFrom<grpc::Arg> for Arg {
    type Error = String;

    fn try_from(request: grpc::Arg) -> Result<Self, Self::Error> {
        let data = request.data.clone();
        let arg = match request.arg_type {
            0 => Arg::Literal(data),
            1 => Arg::FromWorkspace(data),
            _ => return Err("invalid arg_type".to_string()),
        };

        Ok(arg)
    }
}

impl From<Arg> for grpc::Arg {
    fn from(arg: Arg) -> Self {
        let mut result = grpc::Arg::default();

        match arg {
            Arg::Literal(data) => {
                result.arg_type = 0;
                result.data = data;
            },
            Arg::FromWorkspace(data) => {
                result.arg_type = 1;
                result.data = data;
            },
        }

        result
    }
}
