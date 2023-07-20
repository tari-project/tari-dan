//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_dan_app_utilities::template_manager::{implementation::TemplateManager, interface::TemplateManagerError};
use tari_engine_types::instruction::Instruction;
use tari_transaction::Transaction;

use crate::p2p::services::mempool::{AndThen, MempoolError};

#[async_trait]
pub trait Validator<T> {
    type Error;

    async fn validate(&self, input: &T) -> Result<(), Self::Error>;

    fn boxed(self) -> BoxedValidator<T, Self::Error>
    where Self: Sized + Send + Sync + 'static {
        BoxedValidator { inner: Box::new(self) }
    }

    fn and_then<V>(self, other: V) -> AndThen<Self, V>
    where
        V: Validator<T>,
        Self: Sized,
    {
        AndThen::new(self, other)
    }
}

pub struct BoxedValidator<T, E> {
    inner: Box<dyn Validator<T, Error = E> + Send + Sync + 'static>,
}

#[async_trait]
impl<T: Send + Sync, E> Validator<T> for BoxedValidator<T, E> {
    type Error = E;

    async fn validate(&self, input: &T) -> Result<(), Self::Error> {
        self.inner.validate(input).await
    }
}

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
            return Err(MempoolError::NoFeeInstructions);
        }
        Ok(())
    }
}
