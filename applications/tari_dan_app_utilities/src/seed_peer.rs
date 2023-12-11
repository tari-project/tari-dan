//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, str::FromStr};

use anyhow::anyhow;
use libp2p_identity as identity;
use libp2p_identity::PeerId;
use multiaddr::Multiaddr;
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::hex::Hex};

/// Parsed information from a DNS seed record
#[derive(Debug, Clone)]
pub struct SeedPeer {
    pub public_key: RistrettoPublicKey,
    pub addresses: Vec<Multiaddr>,
}

impl SeedPeer {
    pub fn new(public_key: RistrettoPublicKey, addresses: Vec<Multiaddr>) -> Self {
        Self { public_key, addresses }
    }

    pub fn to_peer_id(&self) -> PeerId {
        let pk = identity::PublicKey::from(identity::sr25519::PublicKey::from(self.public_key.clone()));
        pk.to_peer_id()
    }
}

impl FromStr for SeedPeer {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split("::").map(|s| s.trim());
        let public_key = parts
            .next()
            .and_then(|s| RistrettoPublicKey::from_hex(s).ok())
            .ok_or_else(|| anyhow!("Invalid peer id string"))?;
        let addresses = parts.map(Multiaddr::from_str).collect::<Result<Vec<_>, _>>()?;
        if addresses.is_empty() || addresses.iter().any(|a| a.is_empty()) {
            return Err(anyhow!("Empty or invalid address in seed peer string"));
        }
        Ok(SeedPeer { public_key, addresses })
    }
}

impl Display for SeedPeer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}::{}",
            self.public_key,
            self.addresses
                .iter()
                .map(|ma| ma.to_string())
                .collect::<Vec<_>>()
                .join("::")
        )
    }
}
