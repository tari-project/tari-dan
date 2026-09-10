//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use indexmap::IndexSet;
use tari_template_lib::{
    models::ProofId,
    types::{ComponentAddress, NonFungibleAddress, ResourceAddress, TemplateAddress},
};

#[derive(Debug, Clone, Default)]
pub struct AuthParams {
    pub initial_ownership_proofs: IndexSet<NonFungibleAddress>,
}

#[derive(Debug, Clone)]
pub struct AuthorizationScope {
    /// Virtual proofs are system-issued non-fungibles that exist for no longer than the execution e.g. derived from
    /// the transaction signer public key
    virtual_proofs: IndexSet<NonFungibleAddress>,

    /// System-issued badges naming the frame's immediate caller: the caller's component (when it has one) and its
    /// template. Held per frame rather than in the transaction-wide `virtual_proofs` so that a callee only ever sees
    /// the frame directly below it — the badges are re-derived at every push and never merged into a parent or child
    /// scope. There is no path from a badge to a [`ProofId`], so a callee cannot forward the identity it was called
    /// with.
    caller_badges: IndexSet<NonFungibleAddress>,

    /// Resource-based proofs
    proofs: IndexSet<ProofId>,
}

impl AuthorizationScope {
    pub fn new(virtual_proofs: IndexSet<NonFungibleAddress>) -> Self {
        Self {
            virtual_proofs,
            caller_badges: IndexSet::new(),
            proofs: IndexSet::new(),
        }
    }

    pub fn empty() -> Self {
        Self {
            virtual_proofs: IndexSet::new(),
            caller_badges: IndexSet::new(),
            proofs: IndexSet::new(),
        }
    }

    /// Stamps the identity of the frame that is pushing the frame this scope belongs to. The component badge is
    /// omitted for a static function frame, which has no component identity.
    pub(super) fn set_caller(&mut self, component: Option<ComponentAddress>, template: TemplateAddress) {
        self.caller_badges.clear();
        if let Some(component) = component {
            self.caller_badges
                .insert(NonFungibleAddress::caller_component_badge(component));
        }
        self.caller_badges
            .insert(NonFungibleAddress::direct_caller_template_badge(template));
    }

    fn badges(&self) -> impl Iterator<Item = &NonFungibleAddress> {
        self.virtual_proofs.iter().chain(&self.caller_badges)
    }

    pub fn contains_badge(&self, nf_address: &NonFungibleAddress) -> bool {
        self.virtual_proofs.contains(nf_address) || self.caller_badges.contains(nf_address)
    }

    pub fn contains_badge_of_resource(&self, resource_address: &ResourceAddress) -> bool {
        self.badges().any(|badge| badge.resource_address() == resource_address)
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
        for badge in self.badges() {
            write!(f, "{}", badge)?;
        }
        write!(f, "], Proofs: [")?;
        for proof in &self.proofs {
            write!(f, "{}", proof)?;
        }
        write!(f, "]")
    }
}
