//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{atomic::AtomicU32, Arc};

use tari_template_lib::{models::EntityId, Hash};

use crate::hashing::{hasher32, EngineHashDomainLabel};

#[derive(Debug, Clone)]
pub struct EntityIdProvider {
    transaction_hash: Hash,
    max_ids: u32,
    current_id: Arc<AtomicU32>,
}

#[derive(Debug, thiserror::Error)]
pub enum EntityIdProviderError {
    #[error("Maximum ID allocation of {max} exceeded")]
    MaxIdsExceeded { max: u32 },
    #[error("Failed to acquire lock")]
    LockingError { operation: String },
}

impl EntityIdProvider {
    pub fn new(transaction_hash: Hash, max_ids: u32) -> Self {
        Self {
            transaction_hash,
            max_ids,
            current_id: Arc::new(AtomicU32::new(0)),
        }
    }

    fn next(&self) -> Result<u32, EntityIdProviderError> {
        let id = self.current_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if id >= self.max_ids {
            return Err(EntityIdProviderError::MaxIdsExceeded { max: self.max_ids });
        }
        Ok(id)
    }

    pub fn transaction_hash(&self) -> Hash {
        self.transaction_hash
    }

    /// Generates a new entity id trailing_24_bytes(H(tx_hash || n))
    pub fn next_entity_id(&self) -> Result<EntityId, EntityIdProviderError> {
        let id = generate_entity_id(&self.transaction_hash, self.next()?);
        Ok(id)
    }
}

fn generate_entity_id(hash: &Hash, n: u32) -> EntityId {
    let hash = hasher32(EngineHashDomainLabel::EntityId).chain(hash).chain(&n).result();
    EntityId::new(hash.trailing_bytes())
}
