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

use std::{
    borrow::Borrow,
    convert::{TryFrom, TryInto},
};

use anyhow::anyhow;
use chrono::{DateTime, NaiveDateTime, Utc};
use tari_comms::{peer_manager::IdentitySignature, types::CommsPublicKey};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_core::{
    message::{DanMessage, NetworkAnnounce},
    models::TariDanPayload,
};

use crate::p2p::proto;

impl From<DanMessage<TariDanPayload, CommsPublicKey>> for proto::network::DanMessage {
    fn from(msg: DanMessage<TariDanPayload, CommsPublicKey>) -> Self {
        match msg {
            DanMessage::HotStuffMessage(hot_stuff_msg) => Self {
                message: Some(proto::network::dan_message::Message::HotStuff(hot_stuff_msg.into())),
            },
            DanMessage::VoteMessage(vote_msg) => Self {
                message: Some(proto::network::dan_message::Message::Vote(vote_msg.into())),
            },
            DanMessage::NewMempoolMessage(mempool_msg) => Self {
                message: Some(proto::network::dan_message::Message::NewMempoolMessage(
                    mempool_msg.into(),
                )),
            },
            DanMessage::NetworkAnnounce(announce) => Self {
                message: Some(proto::network::dan_message::Message::NetworkAnnounce(announce.into())),
            },
        }
    }
}

impl TryFrom<proto::network::DanMessage> for DanMessage<TariDanPayload, CommsPublicKey> {
    type Error = anyhow::Error;

    fn try_from(value: proto::network::DanMessage) -> Result<Self, Self::Error> {
        let msg_type = value.message.ok_or_else(|| anyhow!("Message type not provided"))?;
        match msg_type {
            proto::network::dan_message::Message::HotStuff(msg) => Ok(DanMessage::HotStuffMessage(msg.try_into()?)),
            proto::network::dan_message::Message::Vote(msg) => Ok(DanMessage::VoteMessage(msg.try_into()?)),
            proto::network::dan_message::Message::NewMempoolMessage(msg) => {
                Ok(DanMessage::NewMempoolMessage(msg.try_into()?))
            },
            proto::network::dan_message::Message::NetworkAnnounce(msg) => {
                Ok(DanMessage::NetworkAnnounce(msg.try_into()?))
            },
        }
    }
}

// -------------------------------- NetworkAnnounce -------------------------------- //

impl<T: ByteArray> From<NetworkAnnounce<T>> for proto::network::NetworkAnnounce {
    fn from(msg: NetworkAnnounce<T>) -> Self {
        Self {
            identity: msg.identity.to_vec(),
            addresses: msg.addresses.into_iter().map(|a| a.to_vec()).collect(),
            identity_signature: Some(msg.identity_signature.into()),
        }
    }
}

impl<T: ByteArray> TryFrom<proto::network::NetworkAnnounce> for NetworkAnnounce<T> {
    type Error = anyhow::Error;

    fn try_from(value: proto::network::NetworkAnnounce) -> Result<Self, Self::Error> {
        Ok(NetworkAnnounce {
            identity: T::from_bytes(&value.identity)?,
            addresses: value
                .addresses
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            identity_signature: value
                .identity_signature
                .ok_or_else(|| anyhow!("Identity signature not provided"))?
                .try_into()?,
        })
    }
}

// -------------------------------- IdentitySignature -------------------------------- //

impl TryFrom<proto::network::IdentitySignature> for IdentitySignature {
    type Error = anyhow::Error;

    fn try_from(value: proto::network::IdentitySignature) -> Result<Self, Self::Error> {
        let version = u8::try_from(value.version).map_err(|_| anyhow!("Invalid identity signature version"))?;
        let signature = value
            .signature
            .map(TryInto::try_into)
            .ok_or_else(|| anyhow!("Signature not provided"))??;
        let updated_at = NaiveDateTime::from_timestamp_opt(value.updated_at, 0)
            .ok_or_else(|| anyhow!("Invalid updated_at timestamp"))?;
        let updated_at = DateTime::<Utc>::from_utc(updated_at, Utc);

        Ok(IdentitySignature::new(version, signature, updated_at))
    }
}

impl<T: Borrow<IdentitySignature>> From<T> for proto::network::IdentitySignature {
    fn from(identity_sig: T) -> Self {
        let sig = identity_sig.borrow();
        proto::network::IdentitySignature {
            version: u32::from(sig.version()),
            signature: Some(sig.signature().into()),
            updated_at: sig.updated_at().timestamp(),
        }
    }
}
