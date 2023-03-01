//  Copyright 2022, The Tari Project
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
use tari_dan_storage::{
    global::{DbEpoch, DbTemplate, DbTemplateUpdate, DbValidatorNode, GlobalDbAdapter, MetadataKey},
    AtomicDb,
};

use crate::storage::StorageError;

#[derive(Debug, Clone, Default)]
pub struct MockGlobalDbBackupAdapter;

impl AtomicDb for MockGlobalDbBackupAdapter {
    type DbTransaction<'a> = ();
    type Error = StorageError;

    fn create_transaction(&self) -> Result<Self::DbTransaction<'_>, Self::Error> {
        todo!()
    }

    fn commit(&self, _transaction: Self::DbTransaction<'_>) -> Result<(), Self::Error> {
        todo!()
    }
}

impl GlobalDbAdapter for MockGlobalDbBackupAdapter {
    fn get_metadata<T: DeserializeOwned>(
        &self,
        _tx: &mut Self::DbTransaction<'_>,
        _key: &MetadataKey,
    ) -> Result<Option<T>, Self::Error> {
        todo!()
    }

    fn set_metadata<T: Serialize>(
        &self,
        _tx: &mut Self::DbTransaction<'_>,
        _key: MetadataKey,
        _value: &T,
    ) -> Result<(), Self::Error> {
        todo!()
    }

    fn get_template(&self, _tx: &mut Self::DbTransaction<'_>, _key: &[u8]) -> Result<Option<DbTemplate>, Self::Error> {
        todo!()
    }

    fn get_templates(&self, _tx: &mut Self::DbTransaction<'_>, _limit: usize) -> Result<Vec<DbTemplate>, Self::Error> {
        todo!()
    }

    fn insert_template(&self, _tx: &mut Self::DbTransaction<'_>, _template: DbTemplate) -> Result<(), Self::Error> {
        todo!()
    }

    fn update_template(
        &self,
        _tx: &mut Self::DbTransaction<'_>,
        _key: &[u8],
        _template: DbTemplateUpdate,
    ) -> Result<(), Self::Error> {
        todo!()
    }

    fn template_exists(&self, _tx: &mut Self::DbTransaction<'_>, _key: &[u8]) -> Result<bool, Self::Error> {
        todo!()
    }

    fn insert_validator_nodes(
        &self,
        _tx: &mut Self::DbTransaction<'_>,
        _validator_nodes: Vec<DbValidatorNode>,
    ) -> Result<(), Self::Error> {
        todo!()
    }

    fn get_validator_nodes_within_epochs(
        &self,
        _tx: &mut Self::DbTransaction<'_>,
        _start_epoch: u64,
        _end_epoch: u64,
    ) -> Result<Vec<DbValidatorNode>, Self::Error> {
        todo!()
    }

    fn get_validator_node(
        &self,
        _tx: &mut Self::DbTransaction<'_>,
        _start_epoch: u64,
        _end_epoch: u64,
        _public_key: &[u8],
    ) -> Result<DbValidatorNode, Self::Error> {
        todo!()
    }

    fn insert_epoch(&self, _tx: &mut Self::DbTransaction<'_>, _epoch: DbEpoch) -> Result<(), Self::Error> {
        todo!()
    }

    fn get_epoch(&self, _tx: &mut Self::DbTransaction<'_>, _epoch: u64) -> Result<Option<DbEpoch>, Self::Error> {
        todo!()
    }
}
