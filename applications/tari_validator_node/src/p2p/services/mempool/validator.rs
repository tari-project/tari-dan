//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_engine_types::instruction::Instruction;
use tari_transaction::Transaction;

use crate::p2p::services::{
    mempool::{MempoolError, Validator},
    template_manager::{TemplateManager, TemplateManagerError},
};

#[derive(Debug)]
pub struct MempoolTransactionValidator {
    template_manager: TemplateManager,
}

impl MempoolTransactionValidator {
    pub(crate) fn new(template_manager: TemplateManager) -> Self {
        Self { template_manager }
    }
}
#[async_trait]
impl Validator<Transaction> for MempoolTransactionValidator {
    type Error = MempoolError;

    async fn validate(&self, inner: &Transaction) -> Result<(), MempoolError> {
        let instructions = inner.instructions();
        for instruction in instructions {
            match instruction {
                Instruction::CallFunction { template_address, .. } => {
                    let template_exists = self.template_manager.template_exists(template_address);
                    match template_exists {
                        Err(e) => return Err(MempoolError::InvalidTemplateAddress(e)),
                        Ok(false) => {
                            return Err(MempoolError::InvalidTemplateAddress(
                                TemplateManagerError::TemplateNotFound {
                                    address: *template_address,
                                },
                            ))
                        },
                        _ => continue,
                    }
                },
                _ => continue,
            }
        }

        Ok(())
    }
}
