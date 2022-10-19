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

use tari_dan_storage::{
    global::{DbTemplate, DbTemplateUpdate, DbValidatorNode, GlobalDbAdapter, MetadataKey},
    AtomicDb,
};

use crate::storage::StorageError;

#[derive(Debug, Clone, Default)]
pub struct MockGlobalDbBackupAdapter;

impl AtomicDb for MockGlobalDbBackupAdapter {
    type DbTransaction = ();
    type Error = StorageError;

    fn create_transaction(&self) -> Result<Self::DbTransaction, Self::Error> {
        todo!()
    }

    fn commit(&self, _transaction: Self::DbTransaction) -> Result<(), Self::Error> {
        todo!()
    }
}

impl GlobalDbAdapter for MockGlobalDbBackupAdapter {
    fn get_metadata(&self, _tx: &Self::DbTransaction, _key: &MetadataKey) -> Result<Option<Vec<u8>>, Self::Error> {
        todo!()
    }

    fn set_metadata(&self, _tx: &Self::DbTransaction, _key: MetadataKey, _value: &[u8]) -> Result<(), Self::Error> {
        todo!()
    }

    fn get_template(&self, _tx: &Self::DbTransaction, _key: &[u8]) -> Result<Option<DbTemplate>, Self::Error> {
        todo!()
    }

    fn get_templates(&self, _tx: &Self::DbTransaction, _limit: usize) -> Result<Vec<DbTemplate>, Self::Error> {
        todo!()
    }

    fn insert_template(&self, _tx: &Self::DbTransaction, _template: DbTemplate) -> Result<(), Self::Error> {
        todo!()
    }

    fn update_template(
        &self,
        _tx: &Self::DbTransaction,
        _key: &[u8],
        _template: DbTemplateUpdate,
    ) -> Result<(), Self::Error> {
        todo!()
    }

    fn insert_validator_nodes(
        &self,
        _tx: &Self::DbTransaction,
        _validator_nodes: Vec<DbValidatorNode>,
    ) -> Result<(), Self::Error> {
        todo!()
    }

    fn get_validator_nodes_per_epoch(
        &self,
        _tx: &Self::DbTransaction,
        _epoch: u64,
    ) -> Result<Vec<DbValidatorNode>, Self::Error> {
        todo!()
    }
}
