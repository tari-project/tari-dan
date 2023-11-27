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

use anyhow::anyhow;
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_crypto::{hashing::DomainSeparation, signatures::SchnorrSignature, tari_utilities::ByteArray};
use tari_dan_common_types::{Epoch, NodeAddressable, ShardId};
use tari_dan_storage::consensus_models::{ValidatorSchnorrSignature, ValidatorSignature};
use tari_transaction::TransactionSignature;

use crate::proto;

//---------------------------------- Signature --------------------------------------------//
impl<H: DomainSeparation> TryFrom<proto::common::Signature> for SchnorrSignature<PublicKey, PrivateKey, H> {
    type Error = anyhow::Error;

    fn try_from(sig: proto::common::Signature) -> Result<Self, Self::Error> {
        let public_nonce = ByteArray::from_canonical_bytes(&sig.public_nonce).map_err(anyhow::Error::msg)?;
        let signature = PrivateKey::from_canonical_bytes(&sig.signature).map_err(anyhow::Error::msg)?;

        Ok(Self::new(public_nonce, signature))
    }
}

impl<H: DomainSeparation> From<SchnorrSignature<PublicKey, PrivateKey, H>> for proto::common::Signature {
    fn from(sig: SchnorrSignature<PublicKey, PrivateKey, H>) -> Self {
        Self {
            public_nonce: sig.get_public_nonce().to_vec(),
            signature: sig.get_signature().to_vec(),
        }
    }
}

impl<TAddr: NodeAddressable> TryFrom<proto::common::SignatureAndPublicKey> for ValidatorSignature<TAddr> {
    type Error = anyhow::Error;

    fn try_from(sig: proto::common::SignatureAndPublicKey) -> Result<Self, Self::Error> {
        let public_key = TAddr::from_bytes(&sig.public_key).ok_or_else(|| anyhow!("Public key was not valid bytes"))?;
        let public_nonce = ByteArray::from_canonical_bytes(&sig.public_nonce).map_err(anyhow::Error::msg)?;
        let signature = PrivateKey::from_canonical_bytes(&sig.signature).map_err(anyhow::Error::msg)?;

        Ok(Self::new(
            public_key,
            ValidatorSchnorrSignature::new(public_nonce, signature),
        ))
    }
}

impl<TAddr: NodeAddressable> From<&ValidatorSignature<TAddr>> for proto::common::SignatureAndPublicKey {
    fn from(value: &ValidatorSignature<TAddr>) -> Self {
        Self {
            public_nonce: value.signature.get_public_nonce().to_vec(),
            signature: value.signature.get_signature().to_vec(),
            public_key: value.public_key.as_bytes().to_vec(),
        }
    }
}

//---------------------------------- TransactionSignature --------------------------------------------//

impl TryFrom<proto::common::SignatureAndPublicKey> for TransactionSignature {
    type Error = anyhow::Error;

    fn try_from(sig: proto::common::SignatureAndPublicKey) -> Result<Self, Self::Error> {
        let public_key = ByteArray::from_canonical_bytes(&sig.public_key).map_err(anyhow::Error::msg)?;
        let public_nonce = ByteArray::from_canonical_bytes(&sig.public_nonce).map_err(anyhow::Error::msg)?;
        let signature = PrivateKey::from_canonical_bytes(&sig.signature).map_err(anyhow::Error::msg)?;

        Ok(Self::new(public_key, Signature::new(public_nonce, signature)))
    }
}

impl From<TransactionSignature> for proto::common::SignatureAndPublicKey {
    fn from(value: TransactionSignature) -> Self {
        Self {
            public_nonce: value.signature().get_public_nonce().to_vec(),
            signature: value.signature().get_signature().to_vec(),
            public_key: value.public_key().to_vec(),
        }
    }
}

// -------------------------------- ShardId -------------------------------- //
impl TryFrom<proto::common::ShardId> for ShardId {
    type Error = anyhow::Error;

    fn try_from(shard_id: proto::common::ShardId) -> Result<Self, Self::Error> {
        Ok(shard_id.bytes.try_into()?)
    }
}

impl From<ShardId> for proto::common::ShardId {
    fn from(shard_id: ShardId) -> Self {
        Self {
            bytes: shard_id.as_bytes().to_vec(),
        }
    }
}

impl From<&ShardId> for proto::common::ShardId {
    fn from(shard_id: &ShardId) -> Self {
        Self {
            bytes: shard_id.as_bytes().to_vec(),
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
