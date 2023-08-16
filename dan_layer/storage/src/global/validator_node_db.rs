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
    collections::{HashMap, HashSet},
    ops::RangeInclusive,
};

use tari_common_types::types::PublicKey;
use tari_dan_common_types::{committee::Committee, shard_bucket::ShardBucket, Epoch, ShardId};

use crate::global::{models::ValidatorNode, GlobalDbAdapter};

pub struct ValidatorNodeDb<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> {
    backend: &'a TGlobalDbAdapter,
    tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
}

impl<'a, 'tx, TGlobalDbAdapter: GlobalDbAdapter> ValidatorNodeDb<'a, 'tx, TGlobalDbAdapter> {
    pub fn new(backend: &'a TGlobalDbAdapter, tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>) -> Self {
        Self { backend, tx }
    }

    pub fn insert_validator_node(
        &mut self,
        public_key: PublicKey,
        shard_key: ShardId,
        epoch: Epoch,
    ) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend
            .insert_validator_node(self.tx, public_key, shard_key, epoch)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn count(&mut self, start_epoch: Epoch, end_epoch: Epoch) -> Result<u64, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_count(self.tx, start_epoch, end_epoch)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn count_in_bucket(
        &mut self,
        start_epoch: Epoch,
        end_epoch: Epoch,
        bucket: ShardBucket,
    ) -> Result<u64, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_count_for_bucket(self.tx, start_epoch, end_epoch, bucket)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get(
        &mut self,
        start_epoch: Epoch,
        end_epoch: Epoch,
        public_key: &[u8],
    ) -> Result<ValidatorNode<PublicKey>, TGlobalDbAdapter::Error> {
        self.backend
            .get_validator_node(self.tx, start_epoch, end_epoch, public_key)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get_all_within_epochs(
        &mut self,
        start_epoch: Epoch,
        end_epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<PublicKey>>, TGlobalDbAdapter::Error> {
        self.backend
            .get_validator_nodes_within_epochs(self.tx, start_epoch, end_epoch)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get_by_shard_range(
        &mut self,
        start_epoch: Epoch,
        end_epoch: Epoch,
        shard_range: RangeInclusive<ShardId>,
    ) -> Result<Vec<ValidatorNode<PublicKey>>, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_get_by_shard_range(self.tx, start_epoch, end_epoch, shard_range)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn get_committees_by_buckets(
        &mut self,
        start_epoch: Epoch,
        end_epoch: Epoch,
        buckets: HashSet<ShardBucket>,
    ) -> Result<HashMap<ShardBucket, Committee<PublicKey>>, TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_get_by_buckets(self.tx, start_epoch, end_epoch, buckets)
            .map_err(TGlobalDbAdapter::Error::into)
    }

    pub fn set_committee_bucket(
        &mut self,
        shard_id: ShardId,
        committee_bucket: ShardBucket,
    ) -> Result<(), TGlobalDbAdapter::Error> {
        self.backend
            .validator_nodes_set_committee_bucket(self.tx, shard_id, committee_bucket)
            .map_err(TGlobalDbAdapter::Error::into)
    }
}
