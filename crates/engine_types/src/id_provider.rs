//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{atomic, atomic::AtomicU32};

use tari_template_lib::{
    models::{BucketId, ProofId},
    types::{
        ComponentAddress,
        ComponentKey,
        EntityId,
        Hash32,
        ObjectKey,
        ResourceAddress,
        TemplateAddress,
        VaultId,
        crypto::RistrettoPublicKeyBytes,
    },
};

use crate::{
    component::derive_component_address_from_public_key,
    hashing::{EngineHashDomainLabel, hasher32},
};

#[derive(Debug, Clone)]
pub struct IdProvider<'a> {
    entity_id: EntityId,
    transaction_hash: Hash32,
    object_ids: &'a ObjectIds,
}

#[derive(Debug, thiserror::Error)]
pub enum IdProviderError {
    #[error("Maximum ID allocation of {max} exceeded")]
    MaxIdsExceeded { max: usize },
    #[error("Failed to acquire lock")]
    LockingError { operation: String },
}

impl<'a> IdProvider<'a> {
    pub fn new(entity_id: EntityId, transaction_hash: Hash32, object_ids: &'a ObjectIds) -> Self {
        Self {
            entity_id,
            transaction_hash,
            object_ids,
        }
    }

    pub fn new_resource_address(&self) -> Result<ResourceAddress, IdProviderError> {
        let key = self.next_object_key()?;
        Ok(ResourceAddress::new(key))
    }

    pub fn new_component_address(&self) -> Result<ComponentAddress, IdProviderError> {
        let component_id = hasher32(EngineHashDomainLabel::ComponentAddress)
            .chain(&self.transaction_hash)
            .chain(&self.next()?)
            .result();

        let object_key = ObjectKey::new(self.entity_id, ComponentKey::new(component_id.trailing_bytes()));
        Ok(ComponentAddress::new(object_key))
    }

    pub fn derive_new_component_address(
        &self,
        template_address: &TemplateAddress,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<ComponentAddress, IdProviderError> {
        Ok(derive_component_address_from_public_key(template_address, public_key))
    }

    pub fn new_vault_id(&self) -> Result<VaultId, IdProviderError> {
        let v = VaultId::new(self.next_object_key()?);
        Ok(v)
    }

    pub fn new_bucket_id(&self) -> BucketId {
        self.object_ids.next_bucket_id()
    }

    pub fn new_proof_id(&self) -> ProofId {
        self.object_ids.next_proof_id()
    }

    pub fn new_uuid(&self, entropy: &[u8]) -> Result<[u8; 32], IdProviderError> {
        let n = self.object_ids.next_uuid_id();
        let h = hasher32(EngineHashDomainLabel::UuidOutput)
            .chain(&self.transaction_hash)
            .chain(&self.entity_id)
            .chain(entropy)
            .chain(&n);
        Ok(h.result().into_array())
    }

    pub fn get_random_bytes(&self, entropy: &[u8], len: usize) -> Result<Vec<u8>, IdProviderError> {
        let mut result = Vec::with_capacity(len);
        while result.len() < len {
            let bytes = self.new_uuid(entropy)?;
            let remaining = len - result.len();
            let end = bytes.len().min(remaining);
            result.extend_from_slice(bytes.get(..end).expect("end bound is always <= length"));
        }

        Ok(result)
    }

    pub fn entity_id(&self) -> EntityId {
        self.entity_id
    }

    fn next(&self) -> Result<u32, IdProviderError> {
        self.object_ids.next_id()
    }

    fn next_object_key(&self) -> Result<ObjectKey, IdProviderError> {
        let hash = generate_output_id(&self.transaction_hash, self.next()?);
        Ok(ObjectKey::new(self.entity_id, ComponentKey::new(hash.trailing_bytes())))
    }
}

fn generate_output_id(transaction_hash: &Hash32, n: u32) -> Hash32 {
    hasher32(EngineHashDomainLabel::Output)
        .chain(transaction_hash)
        .chain(&n)
        .result()
}

#[derive(Debug)]
pub struct ObjectIds {
    max_ids: usize,
    current_id: AtomicU32,
    bucket_id: AtomicU32,
    proof_id: AtomicU32,
    uuid: AtomicU32,
}

impl ObjectIds {
    pub fn new(max_ids: usize) -> Self {
        Self {
            max_ids,
            current_id: AtomicU32::new(0),
            bucket_id: AtomicU32::new(0),
            proof_id: AtomicU32::new(0),
            uuid: AtomicU32::new(0),
        }
    }

    pub fn next_id(&self) -> Result<u32, IdProviderError> {
        let id = self.current_id.fetch_add(1, atomic::Ordering::SeqCst);
        if id as usize >= self.max_ids {
            return Err(IdProviderError::MaxIdsExceeded { max: self.max_ids });
        }
        Ok(id)
    }

    pub fn next_bucket_id(&self) -> BucketId {
        self.bucket_id.fetch_add(1, atomic::Ordering::SeqCst).into()
    }

    pub fn next_proof_id(&self) -> ProofId {
        self.proof_id.fetch_add(1, atomic::Ordering::SeqCst).into()
    }

    pub fn next_uuid_id(&self) -> u32 {
        self.uuid.fetch_add(1, atomic::Ordering::SeqCst)
    }
}

impl Clone for ObjectIds {
    fn clone(&self) -> Self {
        Self {
            max_ids: self.max_ids,
            current_id: AtomicU32::new(self.current_id.load(atomic::Ordering::SeqCst)),
            bucket_id: AtomicU32::new(self.bucket_id.load(atomic::Ordering::SeqCst)),
            proof_id: AtomicU32::new(self.proof_id.load(atomic::Ordering::SeqCst)),
            uuid: AtomicU32::new(self.uuid.load(atomic::Ordering::SeqCst)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_fails_if_generating_more_ids_than_the_max() {
        let object_ids = ObjectIds::new(0);
        let id_provider = IdProvider::new(EntityId::default(), Hash32::default(), &object_ids);
        id_provider.next_object_key().unwrap_err();
        let object_ids = ObjectIds::new(1);
        let id_provider = IdProvider::new(EntityId::default(), Hash32::default(), &object_ids);
        id_provider.next_object_key().unwrap();
        id_provider.next_object_key().unwrap_err();
    }

    #[test]
    fn get_random_bytes() {
        let object_ids = ObjectIds::new(0);
        let id_provider = IdProvider::new(EntityId::default(), Hash32::default(), &object_ids);
        const CASES: [usize; 7] = [0, 4, 32, 33, 64, 65, 129];
        for len in CASES {
            let b = id_provider.get_random_bytes(&[], len).unwrap();
            assert_eq!(b.len(), len);
            if len > 0 {
                assert!(b.iter().any(|&x| x != 0));
            }
        }
    }
}
