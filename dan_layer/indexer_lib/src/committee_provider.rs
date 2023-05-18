//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_dan_common_types::ShardId;
use tari_dan_core::models::Committee;

#[async_trait]
pub trait CommitteeProvider {
    type Addr;
    type Error: std::error::Error + Send + Sync + 'static;

    async fn get_committee(&self, shard_id: ShardId) -> Result<Committee<Self::Addr>, Self::Error>;
    fn get_committee_size(&self) -> usize;
}
