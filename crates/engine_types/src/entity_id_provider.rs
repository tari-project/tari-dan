//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::types::{EntityId, Hash32};

use crate::hashing::{EngineHashDomainLabel, hasher32};

#[derive(Debug)]
pub struct EntityIdProvider {
    transaction_hash: Hash32,
    max_ids: u32,
    current_id: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum EntityIdProviderError {
    #[error("Maximum ID allocation of {max} exceeded")]
    MaxIdsExceeded { max: u32 },
    #[error("Failed to acquire lock")]
    LockingError { operation: String },
}

impl EntityIdProvider {
    pub fn new(transaction_hash: Hash32, max_ids: u32) -> Self {
        Self {
            transaction_hash,
            max_ids,
            current_id: 0,
        }
    }

    fn next(&mut self) -> Result<u32, EntityIdProviderError> {
        let id = self.current_id;
        if id >= self.max_ids {
            return Err(EntityIdProviderError::MaxIdsExceeded { max: self.max_ids });
        }
        self.current_id += 1;
        Ok(id)
    }

    pub fn transaction_hash(&self) -> Hash32 {
        self.transaction_hash
    }

    /// Generates a new entity id trailing_24_bytes(H(tx_hash || n))
    pub fn next_entity_id(&mut self) -> Result<EntityId, EntityIdProviderError> {
        let n = self.next()?;
        let id = generate_entity_id(&self.transaction_hash, n);
        Ok(id)
    }
}

fn generate_entity_id(hash: &Hash32, n: u32) -> EntityId {
    let hash = hasher32(EngineHashDomainLabel::EntityId).chain(hash).chain(&n).result();
    EntityId::new(hash.trailing_bytes())
}
