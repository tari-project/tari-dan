//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[derive(::prost::Message, serde::Serialize, serde::Deserialize)]
pub struct UtxoUpdatePayload {
    #[prost(message, tag = "1")]
    pub sos: Option<StartOfShard>,
    #[prost(oneof = "WalletUtxoUpdate", tags = "2, 3, 4")]
    pub update: Option<WalletUtxoUpdate>,
    #[prost(message, tag = "5")]
    pub eos: Option<EndOfShard>,
}

#[derive(::prost::Message, serde::Serialize, serde::Deserialize)]
pub struct StartOfShard {
    #[prost(uint32, tag = "1")]
    pub shard: u32,
    #[prost(uint64, tag = "2")]
    pub max_state_version: u64,
    #[prost(uint32, tag = "3")]
    pub num_updates: u32,
    /// The shard holds further updates beyond this pass. Authoritative: a pass can stop short of
    /// `per_shard_limit` and still have more to give, so counting `num_updates` cannot answer this.
    #[prost(bool, tag = "4")]
    pub has_more: bool,
}

#[derive(::prost::Message, serde::Serialize, serde::Deserialize)]
pub struct EndOfShard {
    #[prost(uint64, tag = "1")]
    pub max_state_version: u64,
}

#[derive(::prost::Oneof, serde::Serialize, serde::Deserialize)]
pub enum WalletUtxoUpdate {
    #[prost(message, tag = "2")]
    Unspent(UtxoUnspent),
    #[prost(message, tag = "3")]
    Spent(UtxoSpent),
    #[prost(message, tag = "4")]
    Burnt(UtxoBurnt),
}

#[derive(::prost::Message, serde::Serialize, serde::Deserialize)]
pub struct UtxoUnspent {
    #[prost(uint32, tag = "1")]
    pub tag: u32,
    #[prost(bytes, tag = "2")]
    pub public_nonce: Vec<u8>,
}

#[derive(::prost::Message, serde::Serialize, serde::Deserialize)]
pub struct UtxoSpent {
    #[prost(bytes, tag = "1")]
    pub id: Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub version: u64,
}

#[derive(::prost::Message, serde::Serialize, serde::Deserialize)]
pub struct UtxoBurnt {
    #[prost(bytes, tag = "1")]
    pub id: Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub version: u64,
}
