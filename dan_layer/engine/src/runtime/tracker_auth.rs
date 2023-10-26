//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::{component::ComponentHeader, resource::Resource};
use tari_template_lib::auth::{
    AccessRule,
    OwnerRule,
    Ownership,
    RequireRule,
    ResourceAuthAction,
    ResourceOrNonFungibleAddress,
    RestrictedAccessRule,
};

use crate::runtime::{ActionIdent, AuthorizationScope, RuntimeError, StateTracker};

pub struct Authorization<'a> {
    tracker: &'a StateTracker,
}

impl<'a> Authorization<'a> {
    pub fn new(tracker: &'a StateTracker) -> Self {
        Self { tracker }
    }

    pub fn check_component_access_rules(&self, method: &str, component: &ComponentHeader) -> Result<(), RuntimeError> {
        self.tracker.read_with(|state| {
            let scope = &state.current_auth_scope;
            if check_ownership(self.tracker, scope, component.as_ownership())? {
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
                    if !check_restricted_access_rule(self.tracker, scope, rule)? {
                        return Err(RuntimeError::AccessDenied {
                            action_ident: ActionIdent::ComponentCallMethod {
                                method: method.to_string(),
                            },
                        });
                    }

                    Ok(())
                },
            }
        })
    }

    pub fn check_resource_access_rules(
        &self,
        action: ResourceAuthAction,
        resource: &Resource,
    ) -> Result<(), RuntimeError> {
        self.tracker.read_with(|state| {
            let scope = &state.current_auth_scope;
            if check_ownership(self.tracker, scope, resource.as_ownership())? {
                // Owner can invoke any resource method
                return Ok(());
            }

            let rule = resource.access_rules().get_access_rule(&action);
            if !check_access_rule(self.tracker, scope, rule)? {
                return Err(RuntimeError::AccessDenied {
                    action_ident: action.into(),
                });
            }

            Ok(())
        })
    }

    pub fn require_ownership<A: Into<ActionIdent>>(
        &self,
        action: A,
        ownership: Ownership<'_>,
    ) -> Result<(), RuntimeError> {
        self.tracker.read_with(|state| {
            if !check_ownership(self.tracker, &state.current_auth_scope, ownership)? {
                return Err(RuntimeError::AccessDeniedOwnerRequired { action: action.into() });
            }
            Ok(())
        })
    }
}

fn check_ownership(
    tracker: &StateTracker,
    scope: &AuthorizationScope,
    ownership: Ownership<'_>,
) -> Result<bool, RuntimeError> {
    match ownership.owner_rule {
        OwnerRule::OwnedBySigner => {
            let owner_proof = ownership.owner_key.to_non_fungible_address();
            Ok(scope.virtual_proofs().contains(&owner_proof))
        },
        OwnerRule::None => Ok(false),
        OwnerRule::ByAccessRule(rule) => check_access_rule(tracker, scope, rule),
    }
}

fn check_access_rule(
    tracker: &StateTracker,
    scope: &AuthorizationScope,
    rule: &AccessRule,
) -> Result<bool, RuntimeError> {
    match rule {
        AccessRule::AllowAll => Ok(true),
        AccessRule::DenyAll => Ok(false),
        AccessRule::Restricted(rule) => check_restricted_access_rule(tracker, scope, rule),
    }
}

fn check_restricted_access_rule(
    tracker: &StateTracker,
    scope: &AuthorizationScope,
    rule: &RestrictedAccessRule,
) -> Result<bool, RuntimeError> {
    match rule {
        RestrictedAccessRule::Require(rule) => check_require_rule(tracker, scope, rule),
        RestrictedAccessRule::AnyOf(rules) => {
            for rule in rules {
                if check_restricted_access_rule(tracker, scope, rule)? {
                    return Ok(true);
                }
            }
            Ok(false)
        },
        RestrictedAccessRule::AllOf(rules) => {
            for rule in rules {
                if !check_restricted_access_rule(tracker, scope, rule)? {
                    return Ok(false);
                }
            }
            Ok(true)
        },
    }
}

fn check_require_rule(
    tracker: &StateTracker,
    scope: &AuthorizationScope,
    rule: &RequireRule,
) -> Result<bool, RuntimeError> {
    match rule {
        RequireRule::Require(resx_or_addr) => check_resource_or_non_fungible(tracker, scope, resx_or_addr),
        RequireRule::AnyOf(resx_or_addrs) => {
            for resx_or_addr in resx_or_addrs {
                if check_resource_or_non_fungible(tracker, scope, resx_or_addr)? {
                    return Ok(true);
                }
            }

            Ok(false)
        },
        RequireRule::AllOf(resx_or_addr) => {
            for resx_or_addr in resx_or_addr {
                if !check_resource_or_non_fungible(tracker, scope, resx_or_addr)? {
                    return Ok(false);
                }
            }

            Ok(true)
        },
    }
}

fn check_resource_or_non_fungible(
    tracker: &StateTracker,
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
                let matches = tracker.borrow_proof(proof_id, |proof| resx == proof.resource_address())?;

                if matches {
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
                let matches = tracker.borrow_proof(proof_id, |proof| {
                    addr.resource_address() == proof.resource_address() &&
                        proof.non_fungible_token_ids().contains(addr.id())
                })?;

                if matches {
                    return Ok(true);
                }
            }

            Ok(false)
        },
    }
}
