//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

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

#[derive(Debug)]
pub struct IdProvider<'a> {
    entity_id: EntityId,
    transaction_hash: Hash32,
    object_ids: &'a mut ObjectIds,
}

#[derive(Debug, thiserror::Error)]
pub enum IdProviderError {
    #[error("Maximum ID allocation of {max} exceeded")]
    MaxIdsExceeded { max: usize },
    #[error("Failed to acquire lock")]
    LockingError { operation: String },
}

impl<'a> IdProvider<'a> {
    pub fn new(entity_id: EntityId, transaction_hash: Hash32, object_ids: &'a mut ObjectIds) -> Self {
        Self {
            entity_id,
            transaction_hash,
            object_ids,
        }
    }

    pub fn new_resource_address(&mut self) -> Result<ResourceAddress, IdProviderError> {
        let key = self.next_object_key()?;
        Ok(ResourceAddress::new(key))
    }

    pub fn new_component_address(&mut self) -> Result<ComponentAddress, IdProviderError> {
        let n = self.next()?;
        let component_id = hasher32(EngineHashDomainLabel::ComponentAddress)
            .chain(&self.transaction_hash)
            .chain(&n)
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

    pub fn new_vault_id(&mut self) -> Result<VaultId, IdProviderError> {
        let v = VaultId::new(self.next_object_key()?);
        Ok(v)
    }

    pub fn new_bucket_id(&mut self) -> BucketId {
        self.object_ids.next_bucket_id()
    }

    pub fn new_proof_id(&mut self) -> ProofId {
        self.object_ids.next_proof_id()
    }

    pub fn new_uuid(&mut self, entropy: &[u8]) -> Result<[u8; 32], IdProviderError> {
        let n = self.object_ids.next_uuid_id();
        let h = hasher32(EngineHashDomainLabel::UuidOutput)
            .chain(&self.transaction_hash)
            .chain(&self.entity_id)
            .chain(entropy)
            .chain(&n);
        Ok(h.result().into_array())
    }

    pub fn get_random_bytes(&mut self, entropy: &[u8], len: usize) -> Result<Vec<u8>, IdProviderError> {
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

    fn next(&mut self) -> Result<u32, IdProviderError> {
        self.object_ids.next_id()
    }

    fn next_object_key(&mut self) -> Result<ObjectKey, IdProviderError> {
        let n = self.next()?;
        let hash = generate_output_id(&self.transaction_hash, n);
        Ok(ObjectKey::new(self.entity_id, ComponentKey::new(hash.trailing_bytes())))
    }
}

fn generate_output_id(transaction_hash: &Hash32, n: u32) -> Hash32 {
    hasher32(EngineHashDomainLabel::Output)
        .chain(transaction_hash)
        .chain(&n)
        .result()
}

#[derive(Debug, Clone)]
pub struct ObjectIds {
    max_ids: usize,
    current_id: u32,
    /// Buckets and proofs draw from one counter: both are transient handles held in the same runtime
    /// scope, and a shared space keeps a bucket id from ever colliding with a proof id.
    bucket_or_proof_id: u32,
    uuid: u32,
}

impl ObjectIds {
    pub fn new(max_ids: usize) -> Self {
        Self {
            max_ids,
            current_id: 0,
            bucket_or_proof_id: 0,
            uuid: 0,
        }
    }

    pub fn next_id(&mut self) -> Result<u32, IdProviderError> {
        let id = self.current_id;
        if id as usize >= self.max_ids {
            return Err(IdProviderError::MaxIdsExceeded { max: self.max_ids });
        }
        self.current_id += 1;
        Ok(id)
    }

    pub fn next_bucket_id(&mut self) -> BucketId {
        self.next_bucket_or_proof_id().into()
    }

    pub fn next_proof_id(&mut self) -> ProofId {
        self.next_bucket_or_proof_id().into()
    }

    fn next_bucket_or_proof_id(&mut self) -> u32 {
        let id = self.bucket_or_proof_id;
        self.bucket_or_proof_id += 1;
        id
    }

    pub fn next_uuid_id(&mut self) -> u32 {
        let id = self.uuid;
        self.uuid += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_fails_if_generating_more_ids_than_the_max() {
        let mut object_ids = ObjectIds::new(0);
        let mut id_provider = IdProvider::new(EntityId::default(), Hash32::default(), &mut object_ids);
        id_provider.next_object_key().unwrap_err();
        let mut object_ids = ObjectIds::new(1);
        let mut id_provider = IdProvider::new(EntityId::default(), Hash32::default(), &mut object_ids);
        id_provider.next_object_key().unwrap();
        id_provider.next_object_key().unwrap_err();
    }

    #[test]
    fn get_random_bytes() {
        let mut object_ids = ObjectIds::new(0);
        let mut id_provider = IdProvider::new(EntityId::default(), Hash32::default(), &mut object_ids);
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
