//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::{auth::AccessRules, models::NonFungibleAddress};

use crate::runtime::{FunctionIdent, RuntimeError};

#[derive(Debug, Clone)]
pub struct AuthParams {
    pub initial_ownership_proofs: Vec<NonFungibleAddress>,
}

pub struct AuthorizationScope<'a> {
    /// Virtual proofs are system-issued non-fungibles that exist for no longer than the execution e.g. derived from
    /// the transaction sender public key
    virtual_proofs: &'a [NonFungibleAddress],
}

impl<'a> AuthorizationScope<'a> {
    pub fn new(virtual_proofs: &'a [NonFungibleAddress]) -> Self {
        Self { virtual_proofs }
    }

    pub fn check_access_rules(&self, fn_ident: &FunctionIdent, access_rules: &AccessRules) -> Result<(), RuntimeError> {
        match fn_ident {
            FunctionIdent::Native(native_fn) => {
                if access_rules
                    .get_native_access_rule(native_fn)
                    .is_access_allowed(self.virtual_proofs)
                {
                    Ok(())
                } else {
                    Err(RuntimeError::AccessDenied {
                        fn_ident: fn_ident.clone(),
                    })
                }
            },
            FunctionIdent::Template { function, .. } => {
                if access_rules
                    .get_method_access_rule(function)
                    .is_access_allowed(self.virtual_proofs)
                {
                    Ok(())
                } else {
                    Err(RuntimeError::AccessDenied {
                        fn_ident: fn_ident.clone(),
                    })
                }
            },
        }
    }
}
