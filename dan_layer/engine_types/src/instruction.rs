//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use tari_bor::{borsh, Encode};
use tari_common_types::types::{BulletRangeProof, Commitment};
use tari_crypto::{ristretto::RistrettoComSig, signatures::CommitmentSignature};
use tari_template_lib::{
    args::{Arg, LogLevel},
    models::{ComponentAddress, LayerOneCommitmentAddress, TemplateAddress},
    Hash,
};
use tari_utilities::hex::Hex;

use crate::{hashing::hasher, substate::SubstateAddress};

#[derive(Debug, Clone, Encode, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type")]
pub enum Instruction {
    CallFunction {
        template_address: TemplateAddress,
        function: String,
        args: Vec<Arg>,
    },
    CallMethod {
        component_address: ComponentAddress,
        method: String,
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
        commitment_address: LayerOneCommitmentAddress,
        range_proof: BulletRangeProof,
        proof_of_knowledge: RistrettoComSig,
    },
}

impl Instruction {
    pub fn hash(&self) -> Hash {
        hasher("instruction").chain(self).result()
    }
}

impl Display for Instruction {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
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
            Self::ClaimBurn {
                commitment_address,
                proof_of_knowledge,
                ..
            } => {
                write!(
                    f,
                    "ClaimBurn {{ commitment_address: {:?}, proof_of_knowledge: {:?} }}",
                    commitment_address, proof_of_knowledge
                )
            },
        }
    }
}
