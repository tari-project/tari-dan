//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::auth::{
    AccessRule,
    OwnerRule,
    Ownership,
    RequireRule,
    ResourceAuthAction,
    ResourceOrNonFungibleAddress,
    RestrictedAccessRule,
};

use crate::runtime::{
    locking::LockedSubstate,
    working_state::WorkingState,
    ActionIdent,
    AuthorizationScope,
    RuntimeError,
};

pub struct Authorization<'a> {
    state: &'a WorkingState,
}

impl<'a> Authorization<'a> {
    pub(super) fn new(state: &'a WorkingState) -> Self {
        Self { state }
    }

    pub fn check_component_access_rules(&self, method: &str, locked: &LockedSubstate) -> Result<(), RuntimeError> {
        let component = self.state.get_component(locked)?;
        let scope = self.state.current_call_scope()?.auth_scope();
        if check_ownership(self.state, scope, component.as_ownership())? {
            // Owner can call any component method
            return Ok(());
        }

        // Check access rules
        match component.access_rules().get_method_access_rule(method) {
            AccessRule::AllowAll => Ok(()),
            AccessRule::DenyAll => Err(RuntimeError::AccessDenied {
                action_ident: ActionIdent::ComponentCallMethod {
                    method: method.to_string(),
                },
            }),
            AccessRule::Restricted(rule) => {
                if !check_restricted_access_rule(self.state, scope, rule)? {
                    return Err(RuntimeError::AccessDenied {
                        action_ident: ActionIdent::ComponentCallMethod {
                            method: method.to_string(),
                        },
                    });
                }

                Ok(())
            },
        }
    }

    pub fn check_resource_access_rules(
        &self,
        action: ResourceAuthAction,
        locked: &LockedSubstate,
    ) -> Result<(), RuntimeError> {
        let resource = self.state.get_resource(locked)?;
        let scope = self.state.current_call_scope()?.auth_scope();
        if check_ownership(self.state, scope, resource.as_ownership())? {
            // Owner can invoke any resource method
            return Ok(());
        }

        let rule = resource.access_rules().get_access_rule(&action);
        if !check_access_rule(self.state, scope, rule)? {
            return Err(RuntimeError::AccessDenied {
                action_ident: action.into(),
            });
        }

        Ok(())
    }

    pub fn require_ownership<A: Into<ActionIdent>>(
        &self,
        action: A,
        ownership: Ownership<'_>,
    ) -> Result<(), RuntimeError> {
        if !check_ownership(self.state, self.state.current_call_scope()?.auth_scope(), ownership)? {
            return Err(RuntimeError::AccessDeniedOwnerRequired { action: action.into() });
        }
        Ok(())
    }
}

fn check_ownership(
    state: &WorkingState,
    scope: &AuthorizationScope,
    ownership: Ownership<'_>,
) -> Result<bool, RuntimeError> {
    match ownership.owner_rule {
        OwnerRule::OwnedBySigner => {
            let owner_proof = ownership.owner_key.to_non_fungible_address();
            Ok(scope.virtual_proofs().contains(&owner_proof))
        },
        OwnerRule::None => Ok(false),
        OwnerRule::ByAccessRule(rule) => check_access_rule(state, scope, rule),
    }
}

fn check_access_rule(
    state: &WorkingState,
    scope: &AuthorizationScope,
    rule: &AccessRule,
) -> Result<bool, RuntimeError> {
    match rule {
        AccessRule::AllowAll => Ok(true),
        AccessRule::DenyAll => Ok(false),
        AccessRule::Restricted(rule) => check_restricted_access_rule(state, scope, rule),
    }
}

fn check_restricted_access_rule(
    state: &WorkingState,
    scope: &AuthorizationScope,
    rule: &RestrictedAccessRule,
) -> Result<bool, RuntimeError> {
    match rule {
        RestrictedAccessRule::Require(rule) => check_require_rule(state, scope, rule),
        RestrictedAccessRule::AnyOf(rules) => {
            for rule in rules {
                if check_restricted_access_rule(state, scope, rule)? {
                    return Ok(true);
                }
            }
            Ok(false)
        },
        RestrictedAccessRule::AllOf(rules) => {
            for rule in rules {
                if !check_restricted_access_rule(state, scope, rule)? {
                    return Ok(false);
                }
            }
            Ok(true)
        },
    }
}

fn check_require_rule(
    state: &WorkingState,
    scope: &AuthorizationScope,
    rule: &RequireRule,
) -> Result<bool, RuntimeError> {
    match rule {
        RequireRule::Require(resx_or_addr) => check_resource_or_non_fungible(state, scope, resx_or_addr),
        RequireRule::AnyOf(resx_or_addrs) => {
            for resx_or_addr in resx_or_addrs {
                if check_resource_or_non_fungible(state, scope, resx_or_addr)? {
                    return Ok(true);
                }
            }

            Ok(false)
        },
        RequireRule::AllOf(resx_or_addr) => {
            for resx_or_addr in resx_or_addr {
                if !check_resource_or_non_fungible(state, scope, resx_or_addr)? {
                    return Ok(false);
                }
            }

            Ok(true)
        },
    }
}

fn check_resource_or_non_fungible(
    state: &WorkingState,
    scope: &AuthorizationScope,
    resx_or_addr: &ResourceOrNonFungibleAddress,
) -> Result<bool, RuntimeError> {
    match resx_or_addr {
        ResourceOrNonFungibleAddress::Resource(resx) => {
            if scope
                .virtual_proofs()
                .iter()
                .any(|addr| addr.resource_address() == resx)
            {
                return Ok(true);
            }

            for proof_id in scope.proofs() {
                let proof = state.get_proof(*proof_id)?;

                if resx == proof.resource_address() {
                    return Ok(true);
                }
            }
            Ok(false)
        },
        ResourceOrNonFungibleAddress::NonFungibleAddress(addr) => {
            if scope.virtual_proofs().contains(addr) {
                return Ok(true);
            }

            for proof_id in scope.proofs() {
                let proof = state.get_proof(*proof_id)?;

                if addr.resource_address() == proof.resource_address() &&
                    proof.non_fungible_token_ids().contains(addr.id())
                {
                    return Ok(true);
                }
            }

            Ok(false)
        },
    }
}
