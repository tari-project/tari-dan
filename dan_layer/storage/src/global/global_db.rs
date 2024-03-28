//  Copyright 2022. The Tari Project
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

use std::sync::Arc;

use super::{validator_node_db::ValidatorNodeDb, BaseLayerHashesDb, BmtDb, EpochDb};
use crate::{
    global::{backend_adapter::GlobalDbAdapter, metadata_db::MetadataDb, template_db::TemplateDb},
    StorageError,
};

pub trait DbFactory: Sync + Send + 'static {
    type GlobalDbAdapter: GlobalDbAdapter;

    fn get_or_create_global_db(&self) -> Result<GlobalDb<Self::GlobalDbAdapter>, StorageError>;

    fn migrate(&self) -> Result<(), StorageError>;
}

#[derive(Debug, Clone)]
pub struct GlobalDb<TGlobalDbBackendAdapter> {
    adapter: Arc<TGlobalDbBackendAdapter>,
}

impl<TGlobalDbBackendAdapter> GlobalDb<TGlobalDbBackendAdapter> {
    pub fn adapter(&self) -> &TGlobalDbBackendAdapter {
        &self.adapter
    }
}

impl<TGlobalDbAdapter: GlobalDbAdapter> GlobalDb<TGlobalDbAdapter> {
    pub fn new(adapter: TGlobalDbAdapter) -> Self {
        Self {
            adapter: Arc::new(adapter),
        }
    }

    pub fn create_transaction(&self) -> Result<TGlobalDbAdapter::DbTransaction<'_>, TGlobalDbAdapter::Error> {
        let tx = self.adapter.create_transaction()?;
        Ok(tx)
    }

    pub fn templates<'a, 'tx>(
        &'a self,
        tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
    ) -> TemplateDb<'a, 'tx, TGlobalDbAdapter> {
        TemplateDb::new(&self.adapter, tx)
    }

    pub fn metadata<'a, 'tx>(
        &'a self,
        tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
    ) -> MetadataDb<'a, 'tx, TGlobalDbAdapter> {
        MetadataDb::new(&self.adapter, tx)
    }

    pub fn validator_nodes<'a, 'tx>(
        &'a self,
        tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
    ) -> ValidatorNodeDb<'a, 'tx, TGlobalDbAdapter> {
        ValidatorNodeDb::new(&self.adapter, tx)
    }

    pub fn epochs<'a, 'tx>(
        &'a self,
        tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
    ) -> EpochDb<'a, 'tx, TGlobalDbAdapter> {
        EpochDb::new(&self.adapter, tx)
    }

    pub fn base_layer_hashes<'a, 'tx>(
        &'a self,
        tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
    ) -> BaseLayerHashesDb<'a, 'tx, TGlobalDbAdapter> {
        BaseLayerHashesDb::new(&self.adapter, tx)
    }

    pub fn bmt<'a, 'tx>(
        &'a self,
        tx: &'tx mut TGlobalDbAdapter::DbTransaction<'a>,
    ) -> BmtDb<'a, 'tx, TGlobalDbAdapter> {
        BmtDb::new(&self.adapter, tx)
    }

    pub fn commit(&self, tx: TGlobalDbAdapter::DbTransaction<'_>) -> Result<(), TGlobalDbAdapter::Error> {
        self.adapter.commit(tx)?;
        Ok(())
    }
}
