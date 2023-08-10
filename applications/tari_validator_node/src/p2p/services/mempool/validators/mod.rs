//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub use and_then::*;
pub use claim_fee_instructions::*;
pub use fee::*;
pub use input_refs::*;
pub use outputs_dont_exist_locally::*;
pub use template_exists::*;

mod and_then;
mod claim_fee_instructions;
mod fee;
mod input_refs;
mod outputs_dont_exist_locally;
mod template_exists;

use async_trait::async_trait;

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
