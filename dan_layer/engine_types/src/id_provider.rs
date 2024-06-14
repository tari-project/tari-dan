//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::{atomic, atomic::AtomicU32};

use tari_crypto::ristretto::RistrettoPublicKey;
use tari_template_lib::{
    models::{
        BucketId,
        ComponentAddress,
        ComponentKey,
        EntityId,
        ObjectKey,
        ProofId,
        ResourceAddress,
        TemplateAddress,
        VaultId,
    },
    Hash,
};

use crate::{
    component::new_component_address_from_public_key,
    hashing::{hasher32, EngineHashDomainLabel},
};

#[derive(Debug, Clone)]
pub struct IdProvider<'a> {
    entity_id: EntityId,
    transaction_hash: Hash,
    object_ids: &'a ObjectIds,
}

#[derive(Debug, thiserror::Error)]
pub enum IdProviderError {
    #[error("Maximum ID allocation of {max} exceeded")]
    MaxIdsExceeded { max: u32 },
    #[error("Failed to acquire lock")]
    LockingError { operation: String },
}

impl<'a> IdProvider<'a> {
    pub fn new(entity_id: EntityId, transaction_hash: Hash, object_ids: &'a ObjectIds) -> Self {
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

    pub fn new_component_address(
        &self,
        template_address: TemplateAddress,
        public_key_address: Option<RistrettoPublicKey>,
    ) -> Result<ComponentAddress, IdProviderError> {
        if let Some(key) = public_key_address {
            // if a public key address is specified, then it will derive the address from the
            // template hash and public key
            return Ok(new_component_address_from_public_key(&template_address, &key));
        }

        let component_id = hasher32(EngineHashDomainLabel::ComponentAddress)
            .chain(&self.transaction_hash)
            .chain(&self.next()?)
            .result();

        let object_key = ObjectKey::new(self.entity_id, ComponentKey::new(component_id.trailing_bytes()));
        Ok(ComponentAddress::new(object_key))
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

    pub fn new_uuid(&self) -> Result<[u8; 32], IdProviderError> {
        let n = self.object_ids.next_uuid_id();
        let id = hasher32(EngineHashDomainLabel::UuidOutput)
            .chain(&self.transaction_hash)
            .chain(&self.entity_id)
            .chain(&n)
            .result();
        Ok(id.into_array())
    }

    pub fn get_random_bytes(&self, len: usize) -> Result<Vec<u8>, IdProviderError> {
        let mut result = Vec::with_capacity(len);
        while result.len() < len {
            let bytes = self.new_uuid()?;
            let remaining = len - result.len();
            let end = bytes.len().min(remaining);
            result.extend_from_slice(&bytes[..end]);
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

fn generate_output_id(transaction_hash: &Hash, n: u32) -> Hash {
    hasher32(EngineHashDomainLabel::Output)
        .chain(transaction_hash)
        .chain(&n)
        .result()
}

#[derive(Debug)]
pub struct ObjectIds {
    max_ids: u32,
    current_id: AtomicU32,
    bucket_id: AtomicU32,
    uuid: AtomicU32,
}

impl ObjectIds {
    pub fn new(max_ids: u32) -> Self {
        Self {
            max_ids,
            current_id: AtomicU32::new(0),
            bucket_id: AtomicU32::new(0),
            uuid: AtomicU32::new(0),
        }
    }

    pub fn next_id(&self) -> Result<u32, IdProviderError> {
        let id = self.current_id.fetch_add(1, atomic::Ordering::SeqCst);
        if id >= self.max_ids {
            return Err(IdProviderError::MaxIdsExceeded { max: self.max_ids });
        }
        Ok(id)
    }

    pub fn next_bucket_id(&self) -> BucketId {
        self.bucket_id.fetch_add(1, atomic::Ordering::SeqCst).into()
    }

    pub fn next_proof_id(&self) -> ProofId {
        self.bucket_id.fetch_add(1, atomic::Ordering::SeqCst).into()
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
        let id_provider = IdProvider::new(EntityId::default(), Hash::default(), &object_ids);
        id_provider.next_object_key().unwrap_err();
        let object_ids = ObjectIds::new(1);
        let id_provider = IdProvider::new(EntityId::default(), Hash::default(), &object_ids);
        id_provider.next_object_key().unwrap();
        id_provider.next_object_key().unwrap_err();
    }

    #[test]
    fn get_random_bytes() {
        let object_ids = ObjectIds::new(0);
        let id_provider = IdProvider::new(EntityId::default(), Hash::default(), &object_ids);
        const CASES: [usize; 7] = [0, 4, 32, 33, 64, 65, 129];
        for len in CASES {
            let b = id_provider.get_random_bytes(len).unwrap();
            assert_eq!(b.len(), len);
            if len > 0 {
                assert!(b.iter().any(|&x| x != 0));
            }
        }
    }
}
