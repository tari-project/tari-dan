//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_crypto::tari_utilities::hex::Hex;
use tari_template_lib::{
    args::{Arg, LogLevel},
    models::{Amount, ComponentAddress, TemplateAddress},
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    confidential::{ConfidentialClaim, ConfidentialOutput},
    serde_with,
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum Instruction {
    CreateAccount {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        owner_public_key: PublicKey,
        #[cfg_attr(feature = "ts", ts(type = "string | null"))]
        workspace_bucket: Option<String>,
    },
    CallFunction {
        #[serde(with = "serde_with::hex")]
        #[cfg_attr(feature = "ts", ts(type = "Uint8Array"))]
        template_address: TemplateAddress,
        function: String,
        #[serde(deserialize_with = "crate::argument_parser::json_deserialize")]
        args: Vec<Arg>,
    },
    CallMethod {
        #[serde(with = "serde_with::string")]
        component_address: ComponentAddress,
        method: String,
        #[serde(deserialize_with = "crate::argument_parser::json_deserialize")]
        // Argument parser takes an array of strings as input
        #[cfg_attr(feature = "ts", ts(type = "Array<string>"))]
        args: Vec<Arg>,
    },
    PutLastInstructionOutputOnWorkspace {
        key: Vec<u8>,
    },
    EmitLog {
        level: LogLevel,
        message: String,
    },
    ClaimBurn {
        claim: Box<ConfidentialClaim>,
    },
    ClaimValidatorFees {
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        epoch: u64,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        validator_public_key: PublicKey,
    },
    DropAllProofsInWorkspace,
    #[cfg(feature = "debugging")]
    CreateFreeTestCoins {
        revealed_amount: Amount,
        output: Option<ConfidentialOutput>,
    },
}

impl Display for Instruction {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateAccount {
                owner_public_key,
                workspace_bucket,
            } => {
                write!(f, "CreateAccount {{ owner_public_key: {}, bucket: ", owner_public_key,)?;
                match workspace_bucket {
                    Some(bucket) => write!(f, "{}", bucket)?,
                    None => write!(f, "None")?,
                }
                write!(f, " }}")
            },
            Self::CallFunction {
                template_address,
                function,
                args,
            } => write!(
                f,
                "CallFunction {{ template_address: {}, function: {}, args: {:?} }}",
                template_address, function, args
            ),
            Self::CallMethod {
                component_address,
                method,
                args,
            } => write!(
                f,
                "CallMethod {{ component_address: {}, method: {}, args: {:?} }}",
                component_address, method, args
            ),
            Self::PutLastInstructionOutputOnWorkspace { key } => {
                write!(f, "PutLastInstructionOutputOnWorkspace {{ key: {:?} }}", key)
            },
            Self::EmitLog { level, message } => {
                write!(f, "EmitLog {{ level: {:?}, message: {:?} }}", level, message)
            },
            Self::ClaimBurn { claim } => {
                write!(
                    f,
                    "ClaimBurn {{ commitment_address: {}, proof_of_knowledge: nonce({}), u({}) v({}) }}",
                    claim.output_address,
                    claim.proof_of_knowledge.public_nonce().to_hex(),
                    claim.proof_of_knowledge.u().to_hex(),
                    claim.proof_of_knowledge.v().to_hex()
                )
            },
            Self::ClaimValidatorFees {
                epoch,
                validator_public_key,
            } => {
                write!(
                    f,
                    "ClaimValidatorFees {{ epoch: {}, validator_public_key: {:.5} }}",
                    epoch, validator_public_key
                )
            },
            Self::CreateFreeTestCoins { .. } => {
                write!(f, "CreateFreeTestCoins")
            },
            Self::DropAllProofsInWorkspace => {
                write!(f, "DropAllProofsInWorkspace")
            },
        }
    }
}
