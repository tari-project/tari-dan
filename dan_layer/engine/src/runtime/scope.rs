//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use indexmap::IndexSet;
use tari_engine_types::{indexed_value::IndexedWellKnownTypes, lock::LockId, substate::SubstateId, TemplateAddress};
use tari_template_lib::{
    constants::XTR2,
    models::{BucketId, EntityId, ProofId},
    prelude::PUBLIC_IDENTITY_RESOURCE_ADDRESS,
};

use crate::runtime::{
    locking::{LockError, LockedSubstate},
    AuthorizationScope,
    RuntimeError,
};

#[derive(Debug, Clone)]
pub struct CallScope {
    orphans: IndexSet<SubstateId>,
    owned: IndexSet<SubstateId>,
    referenced: IndexSet<SubstateId>,
    component_lock: Option<LockedSubstate>,
    lock_scope: IndexSet<LockId>,
    proof_scope: IndexSet<ProofId>,
    bucket_scope: IndexSet<BucketId>,
    auth_scope: AuthorizationScope,
}

impl CallScope {
    pub fn new() -> Self {
        // Encountered non-determinism bug when using HashSet.
        Self {
            orphans: IndexSet::new(),
            owned: IndexSet::new(),
            referenced: IndexSet::new(),
            component_lock: None,
            lock_scope: IndexSet::new(),
            proof_scope: IndexSet::new(),
            bucket_scope: IndexSet::new(),
            auth_scope: AuthorizationScope::new(vec![]),
        }
    }

    pub fn for_component(component_lock: LockedSubstate) -> Self {
        let mut this = Self::new();
        this.component_lock = Some(component_lock);
        this
    }

    pub(super) fn set_auth_scope(&mut self, scope: AuthorizationScope) {
        self.auth_scope = scope;
    }

    pub fn is_lock_in_scope(&self, lock_id: LockId) -> bool {
        self.lock_scope.contains(&lock_id)
    }

    pub fn lock_scope(&self) -> &IndexSet<LockId> {
        &self.lock_scope
    }

    pub fn is_proof_in_scope(&self, proof_id: ProofId) -> bool {
        self.proof_scope.contains(&proof_id)
    }

    pub fn proof_scope(&self) -> &IndexSet<ProofId> {
        &self.proof_scope
    }

    pub fn bucket_scope(&self) -> &IndexSet<BucketId> {
        &self.bucket_scope
    }

    pub fn is_bucket_in_scope(&self, bucket_id: BucketId) -> bool {
        self.bucket_scope.contains(&bucket_id)
    }

    pub fn is_substate_in_scope(&self, address: &SubstateId) -> bool {
        // TODO: Hacky
        // If the address is the XTR2 resource, it is always in scope
        if *address == XTR2 {
            return true;
        }

        // All Identity resource tokens are in scope
        if address
            .as_non_fungible_address()
            .filter(|addr| *addr.resource_address() == PUBLIC_IDENTITY_RESOURCE_ADDRESS)
            .is_some()
        {
            return true;
        }

        self.owned.contains(address) || self.referenced.contains(address) || self.orphans.contains(address)
    }

    pub fn add_lock_to_scope(&mut self, lock_id: LockId) {
        self.lock_scope.insert(lock_id);
    }

    pub fn add_bucket_to_scope(&mut self, bucket_id: BucketId) {
        self.bucket_scope.insert(bucket_id);
    }

    pub fn remove_bucket_from_scope(&mut self, bucket_id: BucketId) -> bool {
        self.bucket_scope.swap_remove(&bucket_id)
    }

    pub fn add_proof_to_scope(&mut self, proof_id: ProofId) {
        self.proof_scope.insert(proof_id);
        self.auth_scope_mut().add_proof(proof_id);
    }

    pub fn remove_lock_from_scope(&mut self, lock_id: LockId) -> Result<(), RuntimeError> {
        if !self.lock_scope.swap_remove(&lock_id) {
            return Err(RuntimeError::LockError(LockError::LockIdNotFound { lock_id }));
        }
        Ok(())
    }

    pub fn get_current_component_lock(&self) -> Option<&LockedSubstate> {
        self.component_lock.as_ref()
    }

    pub fn owned_nodes(&self) -> &IndexSet<SubstateId> {
        &self.owned
    }

    pub fn orphans(&self) -> &IndexSet<SubstateId> {
        &self.orphans
    }

    pub fn move_node_to_owned(&mut self, address: &SubstateId) -> Result<(), RuntimeError> {
        if self.orphans.swap_remove(address) && !self.owned.insert(address.clone()) {
            return Err(RuntimeError::DuplicateSubstate {
                address: address.clone(),
            });
        }
        Ok(())
    }

    pub fn auth_scope(&self) -> &AuthorizationScope {
        &self.auth_scope
    }

    pub fn auth_scope_mut(&mut self) -> &mut AuthorizationScope {
        &mut self.auth_scope
    }

    pub fn add_substate_to_scope(&mut self, address: SubstateId) -> Result<(), RuntimeError> {
        if self.is_substate_in_scope(&address) {
            return Err(RuntimeError::DuplicateSubstate { address });
        }

        self.add_substate_to_scope_unchecked(address);
        Ok(())
    }

    fn add_substate_to_scope_unchecked(&mut self, address: SubstateId) {
        if address.is_root() {
            self.owned.insert(address);
        } else {
            self.orphans.insert(address);
        }
    }

    /// Add a substate to the owned nodes set without checking if it is already in the scope. This is used when
    /// initializing the root scope from the state store.
    pub fn add_substate_to_owned(&mut self, address: SubstateId) {
        self.referenced.swap_remove(&address);
        self.orphans.swap_remove(&address);
        self.owned.insert(address);
    }

    pub fn add_substate_to_referenced(&mut self, address: SubstateId) {
        if self.is_substate_in_scope(&address) {
            return;
        }
        self.referenced.insert(address);
    }

    pub fn update_from_parent(&mut self, _parent: &CallScope) {
        // Nothing to do? We bring things into scope via the args so that is why we don't need to move things across
        // here.

        // self.owned.extend(_parent.owned.iter().cloned());
        // for proof in _parent.auth_scope.proofs() {
        //     self.auth_scope.add_proof(*proof);
        // }
    }

    pub fn update_from_child_scope(&mut self, child: CallScope) {
        self.owned.extend(child.owned.iter().cloned());
        for owned in &child.owned {
            self.orphans.swap_remove(owned);
        }
        self.proof_scope.extend(child.proof_scope);
        self.bucket_scope.extend(child.bucket_scope);
        self.auth_scope.update_from_child(child.auth_scope);
    }

    pub fn include_in_scope(&mut self, values: &IndexedWellKnownTypes) {
        for addr in values.referenced_substates() {
            // These are never able to be brought into scope
            if addr.is_public_key_identity() || addr.is_vault() || addr.is_transaction_receipt() {
                continue;
            }
            self.add_substate_to_referenced(addr);
        }

        for bucket_id in values.bucket_ids() {
            self.add_bucket_to_scope(*bucket_id);
        }
        for proof_id in values.proof_ids() {
            self.add_proof_to_scope(*proof_id);
        }
    }
}

impl Default for CallScope {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for CallScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.owned.is_empty() {
            writeln!(f, "Owned:")?;
            for owned in &self.owned {
                writeln!(f, "  {}", owned)?;
            }
        }
        if !self.referenced.is_empty() {
            writeln!(f, "Referenced:")?;
            for referenced in &self.referenced {
                writeln!(f, "  {}", referenced)?;
            }
        }
        if !self.orphans.is_empty() {
            writeln!(f, "Orphans:")?;
            for orphan in &self.orphans {
                writeln!(f, "  {}", orphan)?;
            }
        }

        if !self.lock_scope.is_empty() {
            writeln!(f, "Locks:")?;
            for lock in &self.lock_scope {
                writeln!(f, "  {}", lock)?;
            }
        }

        if !self.proof_scope.is_empty() {
            writeln!(f, "Proofs:")?;
            for proof in &self.proof_scope {
                writeln!(f, "  {}", proof)?;
            }
        }

        if !self.bucket_scope.is_empty() {
            writeln!(f, "Buckets:")?;
            for bucket in &self.bucket_scope {
                writeln!(f, "  {}", bucket)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CallFrame {
    scope: CallScope,
    current_template: TemplateAddress,
    current_module: String,
    entity_id: EntityId,
}

impl CallFrame {
    pub fn for_static(current_template: TemplateAddress, current_module: String, entity_id: EntityId) -> Self {
        Self {
            scope: CallScope::new(),
            current_template,
            current_module,
            entity_id,
        }
    }

    pub fn for_component(
        current_template: TemplateAddress,
        current_module: String,
        component_lock: LockedSubstate,
        entity_id: EntityId,
    ) -> Self {
        Self {
            scope: CallScope::for_component(component_lock),
            current_template,
            current_module,
            entity_id,
        }
    }

    pub fn scope(&self) -> &CallScope {
        &self.scope
    }

    pub fn scope_mut(&mut self) -> &mut CallScope {
        &mut self.scope
    }

    pub fn into_scope(self) -> CallScope {
        self.scope
    }

    pub fn entity_id(&self) -> EntityId {
        self.entity_id
    }

    pub fn set_entity_id(&mut self, entity_id: EntityId) {
        self.entity_id = entity_id;
    }

    pub fn current_template(&self) -> (&TemplateAddress, &str) {
        (&self.current_template, &self.current_module)
    }
}

#[derive(Debug, Clone)]
pub enum PushCallFrame {
    ForComponent {
        template_address: TemplateAddress,
        module_name: String,
        component_lock: LockedSubstate,
        arg_scope: IndexedWellKnownTypes,
        entity_id: EntityId,
    },
    Static {
        template_address: TemplateAddress,
        module_name: String,
        arg_scope: IndexedWellKnownTypes,
        entity_id: EntityId,
    },
}

impl PushCallFrame {
    pub fn component_lock(&self) -> Option<&LockedSubstate> {
        match self {
            Self::ForComponent { component_lock, .. } => Some(component_lock),
            Self::Static { .. } => None,
        }
    }

    pub fn arg_scope(&self) -> &IndexedWellKnownTypes {
        match self {
            Self::ForComponent { arg_scope, .. } => arg_scope,
            Self::Static { arg_scope, .. } => arg_scope,
        }
    }

    pub(super) fn into_new_call_frame(self) -> CallFrame {
        match self {
            Self::ForComponent {
                template_address,
                module_name,
                component_lock,
                arg_scope,
                entity_id,
            } => {
                let mut frame = CallFrame::for_component(template_address, module_name, component_lock, entity_id);
                frame.scope_mut().include_in_scope(&arg_scope);
                frame
            },
            Self::Static {
                template_address,
                module_name,
                arg_scope,
                entity_id,
            } => {
                let mut frame = CallFrame::for_static(template_address, module_name, entity_id);
                frame.scope_mut().include_in_scope(&arg_scope);
                frame
            },
        }
    }

    pub fn entity_id(&self) -> Option<EntityId> {
        match self {
            Self::ForComponent { entity_id, .. } => Some(*entity_id),
            Self::Static { .. } => None,
        }
    }
}
