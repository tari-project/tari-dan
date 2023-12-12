//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, hash::Hash};

use libp2p_identity as identity;
use libp2p_identity::PeerId;
use tari_crypto::ristretto::RistrettoPublicKey;

use crate::{DerivableFromPublicKey, NodeAddressable};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct PeerAddress(PeerId);

impl PeerAddress {
    pub fn as_peer_id(&self) -> PeerId {
        self.0
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.to_bytes()
    }
}

impl NodeAddressable for PeerAddress {
    fn zero() -> Self {
        // type, len, data
        Self(PeerId::from_bytes(&[0u8, 1, 0]).unwrap())
    }

    fn try_from_public_key(public_key: &RistrettoPublicKey) -> Option<Self> {
        Some(public_key.clone().into())
    }
}

impl From<identity::PublicKey> for PeerAddress {
    fn from(peer_pk: identity::PublicKey) -> Self {
        peer_pk.to_peer_id().into()
    }
}

impl Display for PeerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_peer_id())
    }
}

impl From<RistrettoPublicKey> for PeerAddress {
    fn from(pk: RistrettoPublicKey) -> Self {
        let pk = identity::PublicKey::from(identity::sr25519::PublicKey::from(pk));
        let peer_id = pk.to_peer_id();
        Self(peer_id)
    }
}

impl From<PeerId> for PeerAddress {
    fn from(peer_id: PeerId) -> Self {
        Self(peer_id)
    }
}

impl From<&PeerId> for PeerAddress {
    fn from(peer_id: &PeerId) -> Self {
        Self(*peer_id)
    }
}

impl PartialEq<PeerId> for PeerAddress {
    fn eq(&self, other: &PeerId) -> bool {
        self.as_peer_id() == *other
    }
}

impl DerivableFromPublicKey for PeerAddress {}

#[cfg(test)]
mod tests {
    use tari_crypto::keys::PublicKey;

    use super::*;

    #[test]
    fn zero() {
        let _ = PeerAddress::zero();
    }

    #[test]
    fn check_conversions() {
        let (_, pk) = RistrettoPublicKey::random_keypair(&mut rand::rngs::OsRng);
        let peer_address = PeerAddress::try_from_public_key(&pk).unwrap();
        let peer_id = peer_address.as_peer_id();
        let peer_address2 = PeerAddress::from(peer_id);
        assert_eq!(peer_address, peer_address2);
        let peer_id2 = peer_address.as_peer_id();
        assert_eq!(peer_id2, peer_id);
    }
}
