//  Copyright 2021. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    collections::{HashMap, HashSet},
    ops::RangeInclusive,
};

use serde::{de::DeserializeOwned, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{committee::Committee, shard_bucket::ShardBucket, Epoch, ShardId};

use super::DbEpoch;
use crate::{
    atomic::AtomicDb,
    global::{
        metadata_db::MetadataKey,
        models::ValidatorNode,
        template_db::{DbTemplate, DbTemplateUpdate},
    },
};

pub trait GlobalDbAdapter: AtomicDb + Send + Sync + Clone {
    fn get_metadata<T: DeserializeOwned>(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: &MetadataKey,
    ) -> Result<Option<T>, Self::Error>;
    fn set_metadata<T: Serialize>(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: MetadataKey,
        value: &T,
    ) -> Result<(), Self::Error>;

    fn template_exists(&self, tx: &mut Self::DbTransaction<'_>, key: &[u8]) -> Result<bool, Self::Error>;

    fn get_template(&self, tx: &mut Self::DbTransaction<'_>, key: &[u8]) -> Result<Option<DbTemplate>, Self::Error>;
    fn get_templates(&self, tx: &mut Self::DbTransaction<'_>, limit: usize) -> Result<Vec<DbTemplate>, Self::Error>;
    fn get_pending_templates(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        limit: usize,
    ) -> Result<Vec<DbTemplate>, Self::Error>;

    fn insert_template(&self, tx: &mut Self::DbTransaction<'_>, template: DbTemplate) -> Result<(), Self::Error>;
    fn update_template(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: &[u8],
        template: DbTemplateUpdate,
    ) -> Result<(), Self::Error>;

    fn insert_validator_node(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        public_key: PublicKey,
        shard_key: ShardId,
        epoch: Epoch,
        fee_claim_public_key: PublicKey,
    ) -> Result<(), Self::Error>;
    fn get_validator_nodes_within_epochs(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<PublicKey>>, Self::Error>;
    fn get_validator_node(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
        public_key: &[u8],
    ) -> Result<ValidatorNode<PublicKey>, Self::Error>;
    fn validator_nodes_count(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
    ) -> Result<u64, Self::Error>;
    fn validator_nodes_count_for_bucket(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
        bucket: ShardBucket,
    ) -> Result<u64, Self::Error>;

    fn validator_nodes_set_committee_bucket(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        shard_key: ShardId,
        bucket: ShardBucket,
    ) -> Result<(), Self::Error>;

    fn validator_nodes_get_by_shard_range(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
        shard_range: RangeInclusive<ShardId>,
    ) -> Result<Vec<ValidatorNode<PublicKey>>, Self::Error>;

    fn validator_nodes_get_by_buckets(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
        buckets: HashSet<ShardBucket>,
    ) -> Result<HashMap<ShardBucket, Committee<PublicKey>>, Self::Error>;

    fn insert_epoch(&self, tx: &mut Self::DbTransaction<'_>, epoch: DbEpoch) -> Result<(), Self::Error>;
    fn get_epoch(&self, tx: &mut Self::DbTransaction<'_>, epoch: u64) -> Result<Option<DbEpoch>, Self::Error>;
}
