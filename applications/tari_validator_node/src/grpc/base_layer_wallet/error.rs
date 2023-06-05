//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHashSizeError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WalletGrpcError {
    #[error("Invalid sig, TODO: fill in deets")]
    InvalidSignature,

    #[error("Metadata was malformed: {0}")]
    MalformedMetadata(String),
    #[error("Could not convert between types:{0}")]
    ConversionError(String),
    #[error("Branched to an unexpected logic path, this is most likely due to a bug:{reason}")]
    InvalidLogicPath { reason: String },
    #[error("Could not decode protobuf message for {message_type}:{source}")]
    ProtoBufDecodeError {
        source: prost::DecodeError,
        message_type: String,
    },
    #[error("Could not encode protobuf message for {message_type}:{source}")]
    ProtoBufEncodeError {
        source: prost::EncodeError,
        message_type: String,
    },
    #[error("Arithmetic overflow")]
    Overflow,
    #[error("Not enough funds")]
    NotEnoughFunds,
    #[error("Entity {entity}:{id} was not found")]
    NotFound { entity: &'static str, id: String },
    #[error("Not authorised: {0}")]
    NotAuthorised(String),
    #[error("Database is missing or has not be created")]
    MissingDatabase,
    #[error("There was no committee for the asset")]
    NoCommitteeForAsset,
    #[error("None of the committee responded")]
    NoResponsesFromCommittee,
    #[error("Fatal error: {0}")]
    FatalError(String),
    #[error("UTXO missing checkpoint data")]
    UtxoNoCheckpointData,
    #[error("Peer did not send a quorum certificate in prepare phase")]
    PreparePhaseNoQuorumCertificate,
    #[error("Quorum certificate does not extend node")]
    PreparePhaseCertificateDoesNotExtendNode,
    #[error("Node not safe")]
    PreparePhaseNodeNotSafe,
    #[error("Connection error: {0}")]
    GrpcConnection(#[from] tonic::transport::Error),
    #[error("GRPC error: {0}")]
    GrpcStatus(#[from] tonic::Status),
    #[error("Failed to decode message: {0}")]
    DecodeError(#[from] prost::DecodeError),
    #[error("Invalid committee public key hex")]
    InvalidCommitteePublicKeyHex,
    #[error("Hash size error: {0}")]
    HashSizeError(#[from] FixedHashSizeError),
    #[error("Failed to register the validator node: {0}")]
    NodeRegistration(String),
}
