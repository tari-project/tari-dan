//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::{
    models::ResourceAddress,
    prelude::{Metadata, NonFungibleId},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NonFungibleToken {
    pub account_address: SubstateAddress,
    pub nft_id: NonFungibleId,
    pub resource_address: ResourceAddress,
    pub token_symbol: String,
    pub metadata: Metadata,
}
