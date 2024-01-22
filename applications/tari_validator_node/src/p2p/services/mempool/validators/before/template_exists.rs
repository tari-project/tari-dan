//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_dan_app_utilities::template_manager::{implementation::TemplateManager, interface::TemplateManagerError};
use tari_dan_common_types::NodeAddressable;
use tari_engine_types::instruction::Instruction;
use tari_transaction::Transaction;

use crate::p2p::services::mempool::{MempoolError, Validator};

#[derive(Debug)]
pub struct TemplateExistsValidator<TAddr> {
    template_manager: TemplateManager<TAddr>,
}

impl<TAddr> TemplateExistsValidator<TAddr> {
    pub(crate) fn new(template_manager: TemplateManager<TAddr>) -> Self {
        Self { template_manager }
    }
}

#[async_trait]
impl<TAddr: NodeAddressable> Validator<Transaction> for TemplateExistsValidator<TAddr> {
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
