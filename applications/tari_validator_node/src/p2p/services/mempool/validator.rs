//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_dan_app_utilities::template_manager::TemplateManagerError;
use tari_engine_types::instruction::Instruction;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_transaction::Transaction;

use crate::p2p::services::{
    mempool::{MempoolError, Validator},
    template_manager::TemplateManager,
};

#[derive(Debug)]
pub struct TemplateExistsValidator {
    template_manager: TemplateManager,
}

impl TemplateExistsValidator {
    pub(crate) fn new(template_manager: TemplateManager) -> Self {
        Self { template_manager }
    }
}

#[async_trait]
impl Validator<Transaction> for TemplateExistsValidator {
    type Error = MempoolError;

    async fn validate(&self, transaction: &Transaction) -> Result<(), MempoolError> {
        let instructions = transaction.instructions();
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

#[derive(Debug)]
pub struct FeeTransactionValidator;

#[async_trait]
impl Validator<Transaction> for FeeTransactionValidator {
    type Error = MempoolError;

    async fn validate(&self, transaction: &Transaction) -> Result<(), MempoolError> {
        if transaction.fee_instructions().is_empty() {
            // Allow 0 fee instructions for account create transactions
            if transaction.instructions().len() == 1 {
                let first = transaction.instructions().first().unwrap();
                let Instruction::CallFunction { template_address, function, args } = first else {
                    return Err(MempoolError::NoFeeInstructions);
                };
                if *template_address == *ACCOUNT_TEMPLATE_ADDRESS && function == "create" && args.len() == 1 {
                    return Ok(());
                }
            }
            return Err(MempoolError::NoFeeInstructions);
        }
        Ok(())
    }
}
