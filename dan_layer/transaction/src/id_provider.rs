//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{atomic::AtomicU32, Arc};

use tari_engine_types::hashing::{hasher, EngineHashDomainLabel};
use tari_template_lib::{
    models::{BucketId, ComponentAddress, ResourceAddress, TemplateAddress, VaultId},
    Hash,
};

#[derive(Debug, Clone)]
pub struct IdProvider {
    transaction_hash: Hash,
    max_ids: u32,
    current_id: Arc<AtomicU32>,
    bucket_id: Arc<AtomicU32>,
    uuid: Arc<AtomicU32>,
}

#[derive(Debug, thiserror::Error)]
#[error("Maximum ID allocation of {max} exceeded")]
pub struct MaxIdsExceeded {
    max: u32,
}

impl IdProvider {
    pub fn new(transaction_hash: Hash, max_ids: u32) -> Self {
        Self {
            transaction_hash,
            max_ids,
            // TODO: these should be ranges
            current_id: Arc::new(AtomicU32::new(0)),
            bucket_id: Arc::new(AtomicU32::new(1000)),
            uuid: Arc::new(AtomicU32::new(0)),
        }
    }

    fn next(&self) -> Result<u32, MaxIdsExceeded> {
        let id = self.current_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if id >= self.max_ids {
            return Err(MaxIdsExceeded { max: self.max_ids });
        }
        Ok(id)
    }

    pub fn transaction_hash(&self) -> Hash {
        self.transaction_hash
    }

    /// Generates a new unique id H(tx_hash || n).
    /// NOTE: we rely on IDs being predictable for all outputs (components, resources, vaults).
    fn new_id(&self) -> Result<Hash, MaxIdsExceeded> {
        let id = generate_output_id(&self.transaction_hash, self.next()?);
        Ok(id)
    }

    pub fn new_resource_address(
        &self,
        template_address: &TemplateAddress,
        token_symbol: &str,
    ) -> Result<ResourceAddress, MaxIdsExceeded> {
        Ok(hasher(EngineHashDomainLabel::ResourceAddress)
            .chain(&template_address)
            .chain(&token_symbol)
            .result()
            .into())
    }

    pub fn new_component_address(&self) -> Result<ComponentAddress, MaxIdsExceeded> {
        Ok(self.new_id()?.into())
    }

    pub fn new_address_hash(&self) -> Result<Hash, MaxIdsExceeded> {
        self.new_id()
    }

    pub fn new_vault_id(&self) -> Result<VaultId, MaxIdsExceeded> {
        Ok(self.new_id()?.into())
    }

    pub fn new_bucket_id(&self) -> BucketId {
        // Buckets are not saved to shards, so should not increment the hashes
        self.bucket_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    pub fn new_uuid(&self) -> Result<[u8; 32], MaxIdsExceeded> {
        let n = self.uuid.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = hasher(EngineHashDomainLabel::UuidOutput)
            .chain(&self.transaction_hash)
            .chain(&n)
            .result();
        Ok(id.into_array())
    }
}

fn generate_output_id(hash: &Hash, n: u32) -> Hash {
    hasher(EngineHashDomainLabel::Output).chain(hash).chain(&n).result()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_fails_if_generating_more_ids_than_the_max() {
        let id_provider = IdProvider::new(Hash::default(), 0);
        id_provider.new_id().unwrap_err();
        let id_provider = IdProvider::new(Hash::default(), 1);
        id_provider.new_id().unwrap();
        id_provider.new_id().unwrap_err();
    }
}
