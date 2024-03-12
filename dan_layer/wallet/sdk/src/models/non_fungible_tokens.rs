//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::serde_with;
use tari_template_lib::{models::VaultId, prelude::NonFungibleId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct NonFungibleToken {
    pub vault_id: VaultId,
    pub nft_id: NonFungibleId,
    #[cfg_attr(feature = "ts", ts(type = "any"))]
    #[serde(with = "serde_with::cbor_value")]
    pub data: tari_bor::Value,
    #[cfg_attr(feature = "ts", ts(type = "any"))]
    #[serde(with = "serde_with::cbor_value")]
    pub mutable_data: tari_bor::Value,
    pub is_burned: bool,
}
