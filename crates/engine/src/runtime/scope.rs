//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use indexmap::IndexSet;
use tari_engine_types::{indexed_value::IndexedWellKnownTypes, lock::LockId, substate::SubstateId};
use tari_template_lib::{
    models::{AddressAllocationId, BucketId, ProofId},
    types::{
        EntityId,
        TemplateAddress,
        constants::{PUBLIC_IDENTITY_RESOURCE_ADDRESS, TARI_TOKEN},
    },
};

use crate::runtime::{
    AuthorizationScope,
    RuntimeError,
    locking::{LockError, LockedSubstate},
};

#[derive(Debug, Clone)]
pub struct CallScope {
    orphans: IndexSet<SubstateId>,
    owned: IndexSet<SubstateId>,
    /// Substates that belong to a component rather than to this frame: seeded at push from the state of the
    /// component the frame executes on, and extended as orphans are attached to a component's state. They are in
    /// scope for this frame alone and are dropped when it is popped, so a component's vaults stay reachable only
    /// from that component.
    component_owned: IndexSet<SubstateId>,
    referenced: IndexSet<SubstateId>,
    /// The proofs this frame holds. `auth_scope.proofs()` is the subset that is currently authorizing an action: a
    /// `ProofAccess` guard leaves that subset when it is dropped while the proof itself is still held, so holding is
    /// tracked separately from authorizing.
    proof_scope: IndexSet<ProofId>,
    /// Buckets and proofs the caller passed in as arguments. They remain the caller's: this frame may leave them
    /// unconsumed, and the caller keeps them once the frame is popped.
    inherited_buckets: IndexSet<BucketId>,
    inherited_proofs: IndexSet<ProofId>,
    component_lock: Option<LockedSubstate>,
    lock_scope: IndexSet<LockId>,
    bucket_scope: IndexSet<BucketId>,
    address_allocation_scope: IndexSet<AddressAllocationId>,
    auth_scope: AuthorizationScope,
}

impl CallScope {
    pub fn new() -> Self {
        // NOTE: HashSet is not appropriate due to non-determinism
        Self {
            orphans: IndexSet::new(),
            owned: IndexSet::new(),
            component_owned: IndexSet::new(),
            referenced: IndexSet::new(),
            proof_scope: IndexSet::new(),
            inherited_buckets: IndexSet::new(),
            inherited_proofs: IndexSet::new(),
            component_lock: None,
            lock_scope: IndexSet::new(),
            bucket_scope: IndexSet::new(),
            address_allocation_scope: IndexSet::new(),
            auth_scope: AuthorizationScope::empty(),
        }
    }

    pub fn for_component(component_lock: LockedSubstate) -> Self {
        let mut this = Self::new();
        this.component_lock = Some(component_lock);
        this
    }

    /// Installs the auth scope this frame starts with. Its proofs came from the frame's caller (the transaction's
    /// base scope for a top-level instruction), so the caller accounts for them.
    pub(crate) fn set_auth_scope(&mut self, scope: AuthorizationScope) {
        self.inherited_proofs.extend(scope.proofs().iter().copied());
        self.proof_scope.extend(scope.proofs().iter().copied());
        self.auth_scope = scope;
    }

    pub fn is_lock_in_scope(&self, lock_id: LockId) -> bool {
        self.lock_scope.contains(&lock_id)
    }

    pub fn lock_scope(&self) -> &IndexSet<LockId> {
        &self.lock_scope
    }

    pub fn is_proof_in_scope(&self, proof_id: &ProofId) -> bool {
        self.proof_scope.contains(proof_id)
    }

    pub fn is_bucket_in_scope(&self, bucket_id: BucketId) -> bool {
        self.bucket_scope.contains(&bucket_id)
    }

    pub fn is_substate_in_scope(&self, address: &SubstateId) -> bool {
        // TODO: Hacky
        // If the address is the XTR resource, it is always in scope
        if *address == TARI_TOKEN {
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

        self.owned.contains(address) ||
            self.component_owned.contains(address) ||
            self.referenced.contains(address) ||
            self.orphans.contains(address)
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

    pub fn is_address_allocation_in_scope(&self, id: AddressAllocationId) -> bool {
        self.address_allocation_scope.contains(&id)
    }

    pub fn add_address_allocation_to_scope(&mut self, id: AddressAllocationId) {
        self.address_allocation_scope.insert(id);
    }

    pub fn remove_address_allocation_from_scope(&mut self, id: AddressAllocationId) -> bool {
        self.address_allocation_scope.swap_remove(&id)
    }

    /// Brings a proof into this frame and authorizes it. A proof reaching a frame — created here, passed in as an
    /// argument, or handed back by a callee — authorizes for the rest of the frame unless the frame drops the
    /// authorization.
    pub fn add_proof_to_scope(&mut self, proof_id: ProofId) {
        self.proof_scope.insert(proof_id);
        self.auth_scope_mut().add_proof(proof_id);
    }

    pub fn remove_proof_from_scope(&mut self, proof_id: &ProofId) {
        self.proof_scope.swap_remove(proof_id);
        self.auth_scope.remove_proof(proof_id);
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

    pub fn take_current_component_lock(&mut self) -> Option<LockedSubstate> {
        self.component_lock.take()
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

    /// Records that an orphan of this frame is now reachable from a component's state, so it stays with the
    /// component when the frame is popped. An address that is already owned — a root id such as a component or
    /// resource the frame created — keeps its place in `owned` and still crosses to the caller; those are gated by
    /// access rules rather than by scope.
    pub fn attach_node_to_component(&mut self, address: &SubstateId) -> Result<(), RuntimeError> {
        if self.orphans.swap_remove(address) && !self.component_owned.insert(address.clone()) {
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
    /// initializing the root scope from the state store and for resources used in buckets.
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

    pub fn remove_substate_from_referenced(&mut self, address: &SubstateId) -> bool {
        self.referenced.swap_remove(address)
    }

    pub fn update_from_parent(&mut self, _parent: &CallScope) {
        // Nothing to do? We bring things into scope via the args so that is why we don't need to move things across
        // here.

        // self.owned.extend(_parent.owned.iter().cloned());
        // for proof in _parent.auth_scope.proofs() {
        //     self.auth_scope.add_proof(*proof);
        // }
    }

    /// Merges what a completed child frame hands back into this scope. Only substates the child created and still
    /// holds loosely, plus the buckets, proofs and address allocations named in its return value, cross the
    /// boundary: everything the child could reach through a component's state stays behind with that component.
    ///
    /// An allocation is as much a capability as a bucket — [`WorkingState::use_allocated_address`] gates on scope
    /// membership alone, and nothing checks that the template consuming an allocation is the one that made it — so
    /// it crosses on the same terms.
    pub fn update_from_child_scope(&mut self, child: CallScope, returned: &IndexedWellKnownTypes) {
        self.owned.extend(child.owned.iter().cloned());
        for owned in &child.owned {
            self.orphans.swap_remove(owned);
        }
        for bucket_id in returned.bucket_ids() {
            if child.bucket_scope.contains(bucket_id) {
                self.bucket_scope.insert(*bucket_id);
            }
        }
        for proof_id in returned.proof_ids() {
            if child.proof_scope.contains(proof_id) {
                self.add_proof_to_scope(*proof_id);
            }
        }
        for allocation in returned.component_address_allocations() {
            if child.address_allocation_scope.contains(&allocation.id()) {
                self.address_allocation_scope.insert(allocation.id());
            }
        }
        for allocation in returned.resource_address_allocations() {
            if child.address_allocation_scope.contains(&allocation.id()) {
                self.address_allocation_scope.insert(allocation.id());
            }
        }
    }

    /// The buckets this frame must account for before it is popped: those it holds that its caller did not lend it.
    pub fn buckets_owed(&self) -> impl Iterator<Item = &BucketId> {
        self.bucket_scope
            .iter()
            .filter(|id| !self.inherited_buckets.contains(*id))
    }

    /// The proofs this frame must account for before it is popped: those it holds and its caller did not lend it.
    /// Read from `proof_scope` rather than the auth scope, so a proof whose `ProofAccess` has been dropped is still
    /// owed.
    pub fn proofs_owed(&self) -> impl Iterator<Item = &ProofId> {
        self.proof_scope
            .iter()
            .filter(|id| !self.inherited_proofs.contains(*id))
    }

    pub fn include_owned_in_scope(&mut self, values: &IndexedWellKnownTypes) {
        for addr in values.referenced_substates() {
            // These are never able to bring these into scope
            if addr.is_virtual() || addr.is_transaction_receipt() || addr.is_template() {
                continue;
            }
            self.component_owned.insert(addr);
        }
    }

    /// Brings what a caller passed in as arguments into this frame. Buckets and proofs stay the caller's — they are
    /// recorded as inherited, so this frame need not account for them — but a proof among them also *authorizes*
    /// here from the moment it arrives, without the frame calling `authorize()` on it. Passing a proof is therefore
    /// the act of lending the authority it carries.
    pub fn include_refs_in_scope(&mut self, values: &IndexedWellKnownTypes) {
        for addr in values.referenced_substates() {
            // Never able to bring these into scope
            if addr.is_virtual() || addr.is_vault() || addr.is_read_only() {
                continue;
            }
            self.add_substate_to_referenced(addr);
        }

        for bucket_id in values.bucket_ids() {
            self.add_bucket_to_scope(*bucket_id);
            self.inherited_buckets.insert(*bucket_id);
        }
        for proof_id in values.proof_ids() {
            self.add_proof_to_scope(*proof_id);
            self.inherited_proofs.insert(*proof_id);
        }
        for allocation in values.component_address_allocations() {
            self.add_address_allocation_to_scope(allocation.id());
        }
        for allocation in values.resource_address_allocations() {
            self.add_address_allocation_to_scope(allocation.id());
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
        if !self.component_owned.is_empty() {
            writeln!(f, "Component owned:")?;
            for owned in &self.component_owned {
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
        if !self.address_allocation_scope.is_empty() {
            writeln!(f, "Address allocations:")?;
            for id in &self.address_allocation_scope {
                writeln!(f, "  {}", id)?;
            }
        }
        Ok(())
    }
}

/// How much of the ledger a call frame may mutate. Ordered from least to most restrictive so that a child frame
/// can never be less restricted than its parent (`FrameWriteMode::max`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FrameWriteMode {
    /// Any substate the frame can lock may be written.
    Full,
    /// Only the component the frame is executing on, through the lock taken at push. Every other write funnelling
    /// through `WorkingState::write_lock_substate` / `new_substate` is rejected, so the frame cannot touch a vault,
    /// resource or any other component. Resource auth hooks run in this mode: the acting component did not choose
    /// the hook code, so the hook must not be able to act on the acting component's behalf beyond its own state.
    OwnComponent,
    /// No state mutation at all. Spend-script predicate frames run in this mode so they are provably
    /// side-effect-free.
    ReadOnly,
}

#[derive(Debug, Clone)]
pub struct CallFrame {
    scope: CallScope,
    current_template: TemplateAddress,
    current_template_name: String,
    entity_id: EntityId,
    allow_cross_template_calls: bool,
    allow_migration_calls: bool,
    write_mode: FrameWriteMode,
}

impl CallFrame {
    pub fn for_static(current_template: TemplateAddress, current_module: String, entity_id: EntityId) -> Self {
        Self {
            scope: CallScope::new(),
            current_template,
            current_template_name: current_module,
            entity_id,
            allow_cross_template_calls: true,
            allow_migration_calls: false,
            write_mode: FrameWriteMode::Full,
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
            current_template_name: current_module,
            entity_id,
            allow_cross_template_calls: true,
            allow_migration_calls: false,
            write_mode: FrameWriteMode::Full,
        }
    }

    pub fn migration_context(
        current_template: TemplateAddress,
        current_module: String,
        component_lock: LockedSubstate,
        entity_id: EntityId,
    ) -> Self {
        Self {
            scope: CallScope::for_component(component_lock),
            current_template,
            current_template_name: current_module,
            entity_id,
            allow_cross_template_calls: false,
            allow_migration_calls: true,
            write_mode: FrameWriteMode::Full,
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

    pub fn current_template(&self) -> &TemplateAddress {
        &self.current_template
    }

    pub fn current_template_name(&self) -> &str {
        &self.current_template_name
    }

    pub fn is_cross_template_calls_allowed(&self) -> bool {
        self.allow_cross_template_calls
    }

    pub fn are_migration_calls_allowed(&self) -> bool {
        self.allow_migration_calls
    }

    pub fn write_mode(&self) -> FrameWriteMode {
        self.write_mode
    }

    /// Restricts this frame to `mode` (never loosening an existing restriction) and disables cross-template
    /// calls. The two restrictions are load-bearing in tandem: the write mode blocks state writes at the lock
    /// layer, while disabling cross-template calls prevents the frame from re-entering other templates, which
    /// would otherwise run with the identity of this frame as their caller.
    pub fn restrict(&mut self, mode: FrameWriteMode) {
        self.write_mode = self.write_mode.max(mode);
        self.allow_cross_template_calls = false;
    }

    /// A frame is never less restricted than the frame that pushed it: a sandboxed frame must not be able to
    /// escape its sandbox by calling into a frame that would then write on its behalf.
    pub fn inherit_restrictions(&mut self, parent: &CallFrame) {
        self.write_mode = self.write_mode.max(parent.write_mode);
        if !parent.allow_cross_template_calls {
            self.allow_cross_template_calls = false;
        }
    }
}

#[derive(Debug)]
pub enum PushCallFrame {
    ForComponent {
        template_address: TemplateAddress,
        module_name: String,
        component_scope: IndexedWellKnownTypes,
        component_lock: LockedSubstate,
        arg_scope: Box<IndexedWellKnownTypes>,
        entity_id: EntityId,
    },
    MigrationContext {
        template_address: TemplateAddress,
        module_name: String,
        component_scope: IndexedWellKnownTypes,
        component_lock: LockedSubstate,
        arg_scope: Box<IndexedWellKnownTypes>,
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
            Self::MigrationContext { component_lock, .. } => Some(component_lock),
            Self::Static { .. } => None,
        }
    }

    pub fn arg_scope(&self) -> &IndexedWellKnownTypes {
        match self {
            Self::ForComponent { arg_scope, .. } => arg_scope,
            Self::MigrationContext { arg_scope, .. } => arg_scope,
            Self::Static { arg_scope, .. } => arg_scope,
        }
    }

    pub(super) fn into_new_call_frame(self) -> CallFrame {
        match self {
            Self::ForComponent {
                template_address,
                module_name,
                component_scope,
                component_lock,
                arg_scope,
                entity_id,
            } => {
                let mut frame = CallFrame::for_component(template_address, module_name, component_lock, entity_id);
                frame.scope_mut().include_owned_in_scope(&component_scope);
                frame.scope_mut().include_refs_in_scope(&arg_scope);
                frame
            },
            Self::MigrationContext {
                template_address,
                module_name,
                component_scope,
                component_lock,
                arg_scope,
                entity_id,
            } => {
                let mut frame = CallFrame::migration_context(template_address, module_name, component_lock, entity_id);
                frame.scope_mut().include_owned_in_scope(&component_scope);
                frame.scope_mut().include_refs_in_scope(&arg_scope);
                frame
            },
            Self::Static {
                template_address,
                module_name,
                arg_scope,
                entity_id,
            } => {
                let mut frame = CallFrame::for_static(template_address, module_name, entity_id);
                frame.scope_mut().include_refs_in_scope(&arg_scope);
                frame
            },
        }
    }

    pub fn entity_id(&self) -> Option<EntityId> {
        match self {
            Self::ForComponent { entity_id, .. } => Some(*entity_id),
            Self::MigrationContext { entity_id, .. } => Some(*entity_id),
            Self::Static { .. } => None,
        }
    }
}
