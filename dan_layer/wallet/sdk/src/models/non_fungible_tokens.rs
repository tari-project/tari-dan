//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::{
    models::VaultId,
    prelude::{Metadata, NonFungibleId},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NonFungibleToken {
    pub vault_id: VaultId,
    pub nft_id: NonFungibleId,
    pub metadata: Metadata,
    pub is_burned: bool,
}
