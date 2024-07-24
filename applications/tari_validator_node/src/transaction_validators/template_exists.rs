//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_dan_app_utilities::template_manager::{implementation::TemplateManager, interface::TemplateManagerError};
use tari_dan_common_types::NodeAddressable;
use tari_engine_types::instruction::Instruction;
use tari_transaction::Transaction;

use crate::{transaction_validators::TransactionValidationError, validator::Validator};

const LOG_TARGET: &str = "tari::dan::mempool::validators::template_exists";

#[derive(Debug)]
pub struct TemplateExistsValidator<TAddr> {
    template_manager: TemplateManager<TAddr>,
}

impl<TAddr> TemplateExistsValidator<TAddr> {
    pub(crate) fn new(template_manager: TemplateManager<TAddr>) -> Self {
        Self { template_manager }
    }
}

impl<TAddr: NodeAddressable> Validator<Transaction> for TemplateExistsValidator<TAddr> {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), TransactionValidationError> {
        let instructions = transaction.instructions();
        for instruction in instructions {
            match instruction {
                Instruction::CallFunction { template_address, .. } => {
                    let template_exists = self.template_manager.template_exists(template_address);
                    match template_exists {
                        Err(e) => return Err(TransactionValidationError::InvalidTemplateAddress(e)),
                        Ok(false) => {
                            warn!(target: LOG_TARGET, "TemplateExistsValidator - FAIL: Template not found");
                            return Err(TransactionValidationError::InvalidTemplateAddress(
                                TemplateManagerError::TemplateNotFound {
                                    address: *template_address,
                                },
                            ));
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
