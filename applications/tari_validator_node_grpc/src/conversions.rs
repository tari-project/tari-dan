use std::{
    borrow::Borrow,
    convert::{TryFrom, TryInto},
};

use borsh::de::BorshDeserialize;
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_dan_engine::instruction::{Instruction, Transaction};
use tari_template_lib::Hash;
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
        let package_id = Hash::deserialize(&mut &request.package_id[..]).map_err(|_| "invalid package_id")?;
        let args = request.args.clone();
        let instruction = match request.instruction_type {
            // function
            0 => {
                let template = request.template;
                let function = request.function;
                Instruction::CallFunction {
                    package_id,
                    template,
                    function,
                    args,
                }
            },
            // method
            1 => {
                let component_id =
                    Hash::deserialize(&mut &request.component_id[..]).map_err(|_| "invalid component_id")?;
                let method = request.method;
                Instruction::CallMethod {
                    package_id,
                    component_id,
                    method,
                    args,
                }
            },
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
                package_id,
                template,
                function,
                args,
            } => {
                result.instruction_type = 0;
                result.package_id = package_id.to_vec();
                result.template = template;
                result.function = function;
                result.args = args;
            },
            Instruction::CallMethod {
                package_id,
                component_id,
                method,
                args,
            } => {
                result.instruction_type = 1;
                result.package_id = package_id.to_vec();
                result.component_id = component_id.to_vec();
                result.method = method;
                result.args = args;
            },
        }

        result
    }
}
