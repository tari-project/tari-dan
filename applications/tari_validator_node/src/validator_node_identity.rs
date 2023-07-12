//   Copyright 2022. The Tari Project
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

use blst::min_sig::{PublicKey as BlsPublicKey, SecretKey as BlsSecretKey};
use serde::{ser::SerializeStruct, Deserialize, Deserializer, Serialize, Serializer};
use tari_comms::NodeIdentity;

#[derive(Clone)]
pub struct ValidatorNodeIdentity {
    node_identity: NodeIdentity,
    consensus_public_key: BlsPublicKey,
    consensus_secret_key: BlsSecretKey,
}

impl ValidatorNodeIdentity {
    pub fn new(node_identity: NodeIdentity, consensus_secret_key: BlsSecretKey) -> Self {
        let consensus_public_key = consensus_secret_key.sk_to_pk();
        Self {
            node_identity,
            consensus_public_key,
            consensus_secret_key,
        }
    }

    pub fn node_identity(&self) -> &NodeIdentity {
        &self.node_identity
    }

    pub fn public_key(&self) -> &BlsPublicKey {
        &self.consensus_public_key
    }
}

impl Serialize for ValidatorNodeIdentity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        let mut state = serializer.serialize_struct("ValidatorNodeIdentity", 3)?;
        state.serialize_field("node_identity", &self.node_identity)?;
        state.serialize_field("consensus_public_key", &self.consensus_public_key.to_bytes().to_vec())?;
        state.serialize_field("consensus_secret_key", &self.consensus_secret_key.to_bytes().to_vec())?;

        state.end()
    }
}

impl<'de> Deserialize<'de> for ValidatorNodeIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: Deserializer<'de> {
        struct ValidatorNodeIdentityVisitor;

        impl<'de> serde::de::Visitor<'de> for ValidatorNodeIdentityVisitor {
            type Value = ValidatorNodeIdentity;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct ValidatorNodeIdentity")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where M: serde::de::MapAccess<'de> {
                let node_identity = map
                    .next_entry::<&str, NodeIdentity>()?
                    .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?
                    .1;
                let consensus_public_key_bytes = map
                    .next_entry::<&str, Vec<u8>>()?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?
                    .1;
                let consensus_secret_key_bytes = map
                    .next_entry::<&str, Vec<u8>>()?
                    .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?
                    .1;

                let consensus_public_key = BlsPublicKey::from_bytes(&consensus_public_key_bytes).map_err(|e| {
                    serde::de::Error::custom(format!("Failed to deserialize consensus public key: {:?}", e))
                })?;
                let consensus_secret_key = BlsSecretKey::from_bytes(&consensus_secret_key_bytes).map_err(|e| {
                    serde::de::Error::custom(format!("Failed to deserialize consensus secret key: {:?}", e))
                })?;

                Ok(ValidatorNodeIdentity {
                    node_identity,
                    consensus_public_key,
                    consensus_secret_key,
                })
            }
        }

        deserializer.deserialize_map(ValidatorNodeIdentityVisitor)
    }
}

#[cfg(test)]
mod tests {
    use tari_comms::{multiaddr::Multiaddr, peer_manager::PeerFeatures};

    use super::*;

    #[test]
    fn test_deserialize_serialize() {
        let random_seed: [u8; 32] = rand::random();
        let secret_key =
            BlsSecretKey::key_gen(&random_seed, &[]).expect("Failed to generate secret key from random material");
        let public_key = secret_key.sk_to_pk();

        let node_identity = NodeIdentity::random(&mut rand::rngs::OsRng, Multiaddr::empty(), PeerFeatures::default());
        let validator_node_identity = ValidatorNodeIdentity::new(node_identity, secret_key);

        // Serialize the ValidatorNodeIdentity instance to JSON
        let serialized =
            serde_json::to_string(&validator_node_identity).expect("Failed to serialize validator node identity");

        println!("serialized = {}", serialized);

        // Deserialize the JSON back to a ValidatorNodeIdentity instance
        let deserialized: ValidatorNodeIdentity =
            serde_json::from_str(&serialized).expect("Failed to deserialize validator node identity");
    }
}
