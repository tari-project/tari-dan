//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, str::FromStr};

use serde::{Deserialize, Serialize};
use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

use crate::Epoch;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerOneTransactionDef<T> {
    pub payload_type: LayerOnePayloadType,
    pub payload: T,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LayerOnePayloadType {
    /// Payload is a ValidatorRegistrationParams
    ValidatorRegistration,
    /// Payload is a ValidatorExitParams
    ValidatorExit,
}

impl LayerOnePayloadType {
    pub fn is_validator_registration(&self) -> bool {
        matches!(self, Self::ValidatorRegistration)
    }
}

impl FromStr for LayerOnePayloadType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("validatorregistration") {
            return Ok(Self::ValidatorRegistration);
        }
        if s.eq_ignore_ascii_case("validatorexit") {
            return Ok(Self::ValidatorExit);
        }
        Err(format!("Invalid LayerOnePayloadType: {}", s))
    }
}

impl Display for LayerOnePayloadType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ValidatorRegistration => write!(f, "ValidatorRegistration"),
            Self::ValidatorExit => write!(f, "ValidatorExit"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorRegistrationParams {
    pub sidechain_public_key: Option<RistrettoPublicKeyBytes>,
    pub public_key: RistrettoPublicKeyBytes,
    pub signature: SchnorrSignatureBytes,
    pub claim_public_key: RistrettoPublicKeyBytes,
    pub max_epoch: Epoch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorExitParams {
    pub sidechain_public_key: Option<RistrettoPublicKeyBytes>,
    pub public_key: RistrettoPublicKeyBytes,
    pub signature: SchnorrSignatureBytes,
    pub max_epoch: Epoch,
}
