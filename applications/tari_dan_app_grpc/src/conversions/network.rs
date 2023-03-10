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
    workers::hotstuff_waiter::RecoveryMessage,
};

use crate::proto;

impl From<DanMessage<TariDanPayload, CommsPublicKey>> for proto::network::DanMessage {
    fn from(msg: DanMessage<TariDanPayload, CommsPublicKey>) -> Self {
        let message_tag = msg.get_message_tag();
        match msg {
            DanMessage::HotStuffMessage(hot_stuff_msg) => Self {
                message: Some(proto::network::dan_message::Message::HotStuff((*hot_stuff_msg).into())),
                message_tag,
            },
            DanMessage::VoteMessage(vote_msg) => Self {
                message: Some(proto::network::dan_message::Message::Vote(vote_msg.into())),
                message_tag,
            },
            DanMessage::NewTransaction(transaction) => Self {
                message: Some(proto::network::dan_message::Message::NewTransaction(
                    (*transaction).into(),
                )),
                message_tag,
            },
            DanMessage::NetworkAnnounce(announce) => Self {
                message: Some(proto::network::dan_message::Message::NetworkAnnounce(
                    (*announce).into(),
                )),
                message_tag,
            },
            DanMessage::RecoveryMessage(recovery_msg) => Self {
                message: Some(proto::network::dan_message::Message::RecoveryMessage(
                    (*recovery_msg).into(),
                )),
                message_tag,
            },
        }
    }
}

impl TryFrom<proto::network::DanMessage> for DanMessage<TariDanPayload, CommsPublicKey> {
    type Error = anyhow::Error;

    fn try_from(value: proto::network::DanMessage) -> Result<Self, Self::Error> {
        let msg_type = value.message.ok_or_else(|| anyhow!("Message type not provided"))?;
        match msg_type {
            proto::network::dan_message::Message::HotStuff(msg) => {
                Ok(DanMessage::HotStuffMessage(Box::new(msg.try_into()?)))
            },
            proto::network::dan_message::Message::Vote(msg) => Ok(DanMessage::VoteMessage(msg.try_into()?)),
            proto::network::dan_message::Message::NewTransaction(msg) => {
                Ok(DanMessage::NewTransaction(Box::new(msg.try_into()?)))
            },
            proto::network::dan_message::Message::NetworkAnnounce(msg) => {
                Ok(DanMessage::NetworkAnnounce(Box::new(msg.try_into()?)))
            },
            proto::network::dan_message::Message::RecoveryMessage(msg) => {
                Ok(DanMessage::RecoveryMessage(Box::new(msg.try_into()?)))
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
        let identity_signature = value
            .identity_signature
            .ok_or_else(|| anyhow!("Identity signature not provided"))?;
        Ok(NetworkAnnounce {
            identity: T::from_bytes(&value.identity)?,
            addresses: value
                .addresses
                .into_iter()
                .map(|a| a.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            identity_signature: IdentitySignature::new(
                identity_signature.version,
                identity_signature.signature,
                identity_signature.updated_at,
            ),
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

// -------------------------------- RecoveryMessage -------------------------------- //
impl TryFrom<proto::network::RecoveryMessage> for RecoveryMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::network::RecoveryMessage) -> Result<Self, Self::Error> {
        let msg_type = value.message.ok_or_else(|| anyhow!("Message type not provided"))?;
        match msg_type {
            proto::network::recovery_message::Message::MissingProposal(msg) => Ok(RecoveryMessage::MissingProposal(
                msg.epoch.into(),
                msg.shard_id.try_into()?,
                msg.payload_id.try_into()?,
                msg.last_known_height.into(),
            )),
            proto::network::recovery_message::Message::ElectionInProgress(msg) => {
                Ok(RecoveryMessage::ElectionInProgress(
                    msg.epoch.into(),
                    msg.shard_id.try_into()?,
                    msg.payload_id.try_into()?,
                ))
            },
        }
    }
}

impl From<RecoveryMessage> for proto::network::RecoveryMessage {
    fn from(value: RecoveryMessage) -> Self {
        match value {
            RecoveryMessage::MissingProposal(epoch, shard_id, payload_id, last_known_height) => {
                proto::network::RecoveryMessage {
                    message: Some(proto::network::recovery_message::Message::MissingProposal(
                        proto::network::MissingProposal {
                            epoch: epoch.0,
                            shard_id: shard_id.into(),
                            payload_id: payload_id.as_bytes().into(),
                            last_known_height: last_known_height.0,
                        },
                    )),
                }
            },
            RecoveryMessage::ElectionInProgress(epoch, shard_id, payload_id) => proto::network::RecoveryMessage {
                message: Some(proto::network::recovery_message::Message::ElectionInProgress(
                    proto::network::ElectionInProgress {
                        epoch: epoch.0,
                        shard_id: shard_id.into(),
                        payload_id: payload_id.as_bytes().into(),
                    },
                )),
            },
        }
    }
}
