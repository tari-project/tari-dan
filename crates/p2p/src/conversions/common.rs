//   Copyright 2023. The Tari Project
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

use std::convert::{TryFrom, TryInto};

use anyhow::Context;
use tari_consensus_types::ValidatorSignatureBytes;
use tari_crypto::tari_utilities::ByteArray;
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{Epoch, SubstateAddress, SubstateRequirement, SubstateRequirementRef, SubstateVersion};
use tari_template_lib::types::{
    Amount,
    crypto::{
        CommitmentSignatureBytes,
        PedersenCommitmentBytes,
        RistrettoPublicKeyBytes,
        Scalar32Bytes,
        SchnorrSignatureBytes,
    },
};

use crate::{proto, proto::common::OptionalVersion};

//---------------------------------- Signature --------------------------------------------//
impl TryFrom<proto::common::Signature> for SchnorrSignatureBytes {
    type Error = anyhow::Error;

    fn try_from(sig: proto::common::Signature) -> Result<Self, Self::Error> {
        let public_nonce = RistrettoPublicKeyBytes::from_bytes(&sig.public_nonce).map_err(anyhow::Error::msg)?;
        let signature = Scalar32Bytes::from_bytes(&sig.signature).map_err(anyhow::Error::msg)?;

        Ok(Self::new(public_nonce, signature))
    }
}

impl From<SchnorrSignatureBytes> for proto::common::Signature {
    fn from(sig: SchnorrSignatureBytes) -> Self {
        (&sig).into()
    }
}

impl From<&SchnorrSignatureBytes> for proto::common::Signature {
    fn from(sig: &SchnorrSignatureBytes) -> Self {
        Self {
            public_nonce: sig.public_nonce().to_vec(),
            signature: sig.signature().to_vec(),
        }
    }
}

impl TryFrom<proto::common::SignatureAndPublicKey> for ValidatorSignatureBytes {
    type Error = anyhow::Error;

    fn try_from(sig: proto::common::SignatureAndPublicKey) -> Result<Self, Self::Error> {
        let public_key =
            RistrettoPublicKeyBytes::from_bytes(&sig.public_key).context("Public key was not valid bytes")?;
        let public_nonce =
            RistrettoPublicKeyBytes::from_bytes(&sig.public_nonce).context("public nonce bytes length != 32")?;
        let signature = Scalar32Bytes::from_bytes(&sig.signature).context("signature bytes length != 32")?;

        Ok(Self::new(
            public_key,
            SchnorrSignatureBytes::new(public_nonce, signature),
        ))
    }
}

impl From<&ValidatorSignatureBytes> for proto::common::SignatureAndPublicKey {
    fn from(value: &ValidatorSignatureBytes) -> Self {
        Self {
            public_nonce: value.signature.public_nonce().to_vec(),
            signature: value.signature.signature().to_vec(),
            public_key: value.public_key.as_bytes().to_vec(),
        }
    }
}

// -------------------------------- SubstateAddress -------------------------------- //
impl TryFrom<proto::common::SubstateAddress> for SubstateAddress {
    type Error = anyhow::Error;

    fn try_from(address: proto::common::SubstateAddress) -> Result<Self, Self::Error> {
        Ok(address.bytes.try_into()?)
    }
}

impl From<SubstateAddress> for proto::common::SubstateAddress {
    fn from(address: SubstateAddress) -> Self {
        Self {
            bytes: address.as_bytes().to_vec(),
        }
    }
}

impl From<&SubstateAddress> for proto::common::SubstateAddress {
    fn from(address: &SubstateAddress) -> Self {
        Self {
            bytes: address.as_bytes().to_vec(),
        }
    }
}

//---------------------------------- Epoch --------------------------------------------//
impl From<proto::common::Epoch> for Epoch {
    fn from(epoch: proto::common::Epoch) -> Self {
        Epoch(epoch.epoch)
    }
}

impl From<Epoch> for proto::common::Epoch {
    fn from(epoch: Epoch) -> Self {
        Self { epoch: epoch.as_u64() }
    }
}

//---------------------------------- Amount --------------------------------------------//

impl From<proto::common::Amount> for Amount {
    fn from(value: proto::common::Amount) -> Self {
        let digits = [value.digit1, value.digit2];
        Self::from_le_digits(digits)
    }
}

impl From<Amount> for proto::common::Amount {
    fn from(value: Amount) -> Self {
        let digits = value.to_le_digits();
        Self {
            digit1: digits[0],
            digit2: digits[1],
        }
    }
}
// -------------------------------- CommitmentSignature -------------------------------- //

impl TryFrom<proto::common::CommitmentSignature> for CommitmentSignatureBytes {
    type Error = anyhow::Error;

    fn try_from(val: proto::common::CommitmentSignature) -> Result<Self, Self::Error> {
        let u = Scalar32Bytes::from_bytes(&val.signature_u).context("Invalid u signature")?;
        let v = Scalar32Bytes::from_bytes(&val.signature_v).context("Invalid v signature")?;
        let public_nonce =
            PedersenCommitmentBytes::from_bytes(&val.public_nonce_commitment).context("Invalid public nonce")?;

        Ok(Self::new(public_nonce, u, v))
    }
}

impl From<CommitmentSignatureBytes> for proto::common::CommitmentSignature {
    fn from(val: CommitmentSignatureBytes) -> Self {
        Self {
            public_nonce_commitment: val.public_nonce().to_vec(),
            signature_u: val.u().to_vec(),
            signature_v: val.v().to_vec(),
        }
    }
}

// -------------------------------- SubstateRequirement -------------------------------- //

impl TryFrom<proto::common::SubstateRequirement> for SubstateRequirement {
    type Error = anyhow::Error;

    fn try_from(val: proto::common::SubstateRequirement) -> Result<Self, Self::Error> {
        let substate_id = SubstateId::from_bytes(&val.substate_id)?;
        let version = val.version.map(|v| SubstateVersion::new(v.version));
        let substate_specification = SubstateRequirement::new(substate_id, version);
        Ok(substate_specification)
    }
}

impl From<SubstateRequirement> for proto::common::SubstateRequirement {
    fn from(val: SubstateRequirement) -> Self {
        (&val).into()
    }
}

impl From<&SubstateRequirement> for proto::common::SubstateRequirement {
    fn from(val: &SubstateRequirement) -> Self {
        Self {
            substate_id: val.substate_id().to_bytes(),
            version: val.version().map(|v| OptionalVersion { version: v.as_u64() }),
        }
    }
}
impl From<SubstateRequirementRef<'_>> for proto::common::SubstateRequirement {
    fn from(val: SubstateRequirementRef<'_>) -> Self {
        Self {
            substate_id: val.substate_id().to_bytes(),
            version: val.version().map(|v| OptionalVersion { version: v.as_u64() }),
        }
    }
}
