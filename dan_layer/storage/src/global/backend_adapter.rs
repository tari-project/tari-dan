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

use serde::{de::DeserializeOwned, Serialize};

use super::{validator_node_db::DbValidatorNode, DbEpoch};
use crate::{
    atomic::AtomicDb,
    global::{
        metadata_db::MetadataKey,
        template_db::{DbTemplate, DbTemplateUpdate},
    },
};

pub trait GlobalDbAdapter: AtomicDb + Send + Sync + Clone {
    fn get_metadata<T: DeserializeOwned>(
        &self,
        tx: &Self::DbTransaction<'_>,
        key: &MetadataKey,
    ) -> Result<Option<T>, Self::Error>;
    fn set_metadata<T: Serialize>(
        &self,
        tx: &Self::DbTransaction<'_>,
        key: MetadataKey,
        value: &T,
    ) -> Result<(), Self::Error>;

    fn get_template(&self, tx: &Self::DbTransaction<'_>, key: &[u8]) -> Result<Option<DbTemplate>, Self::Error>;
    fn get_templates(&self, tx: &Self::DbTransaction<'_>, limit: usize) -> Result<Vec<DbTemplate>, Self::Error>;

    fn insert_template(&self, tx: &Self::DbTransaction<'_>, template: DbTemplate) -> Result<(), Self::Error>;
    fn update_template(
        &self,
        tx: &Self::DbTransaction<'_>,
        key: &[u8],
        template: DbTemplateUpdate,
    ) -> Result<(), Self::Error>;

    fn insert_validator_nodes(
        &self,
        tx: &Self::DbTransaction<'_>,
        validator_nodes: Vec<DbValidatorNode>,
    ) -> Result<(), Self::Error>;
    fn get_validator_nodes_within_epochs(
        &self,
        tx: &Self::DbTransaction<'_>,
        start_epoch: u64,
        end_epoch: u64,
    ) -> Result<Vec<DbValidatorNode>, Self::Error>;

    fn get_validator_node(
        &self,
        tx: &Self::DbTransaction<'_>,
        start_epoch: u64,
        end_epoch: u64,
        public_key: &[u8],
    ) -> Result<DbValidatorNode, Self::Error>;
    fn insert_epoch(&self, tx: &Self::DbTransaction<'_>, epoch: DbEpoch) -> Result<(), Self::Error>;
    fn get_epoch(&self, tx: &Self::DbTransaction<'_>, epoch: u64) -> Result<Option<DbEpoch>, Self::Error>;
}
