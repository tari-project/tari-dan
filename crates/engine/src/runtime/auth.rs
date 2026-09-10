//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, sync::Arc};

use indexmap::IndexSet;
use tari_template_lib::{
    models::ProofId,
    types::{NonFungibleAddress, ResourceAddress},
};

#[derive(Debug, Clone, Default)]
pub struct AuthParams {
    pub initial_ownership_proofs: Arc<IndexSet<NonFungibleAddress>>,
}

#[derive(Debug, Clone)]
pub struct AuthorizationScope {
    /// Virtual proofs are system-issued non-fungibles that exist for no longer than the execution e.g. derived from
    /// the transaction signer public key
    virtual_proofs: Arc<IndexSet<NonFungibleAddress>>,

    /// Resource-based proofs
    proofs: IndexSet<ProofId>,
}

impl AuthorizationScope {
    pub fn new(virtual_proofs: Arc<IndexSet<NonFungibleAddress>>) -> Self {
        Self {
            virtual_proofs,
            proofs: IndexSet::new(),
        }
    }

    pub fn empty() -> Self {
        Self {
            virtual_proofs: Arc::new(IndexSet::new()),
            proofs: IndexSet::new(),
        }
    }

    pub fn virtual_proofs(&self) -> &IndexSet<NonFungibleAddress> {
        &self.virtual_proofs
    }

    pub fn contains_badge(&self, nf_address: &NonFungibleAddress) -> bool {
        self.virtual_proofs.contains(nf_address)
    }

    pub fn contains_badge_of_resource(&self, resource_address: &ResourceAddress) -> bool {
        self.virtual_proofs
            .iter()
            .any(|badge| badge.resource_address() == resource_address)
    }

    pub fn proofs(&self) -> &IndexSet<ProofId> {
        &self.proofs
    }

    pub fn add_proof(&mut self, proof_id: ProofId) {
        self.proofs.insert(proof_id);
    }

    pub fn remove_proof(&mut self, proof_id: &ProofId) -> bool {
        self.proofs.swap_remove(proof_id)
    }

    pub fn contains_proof(&self, proof_id: &ProofId) -> bool {
        self.proofs.contains(proof_id)
    }
}

impl Display for AuthorizationScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Virtual: [")?;
        for proof in self.virtual_proofs.iter() {
            write!(f, "{}", proof)?;
        }
        write!(f, "], Proofs: [")?;
        for proof in &self.proofs {
            write!(f, "{}", proof)?;
        }
        write!(f, "]")
    }
}
