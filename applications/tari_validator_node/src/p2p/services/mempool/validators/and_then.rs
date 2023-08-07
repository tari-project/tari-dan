//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;

use super::Validator;

pub struct AndThen<A, U> {
    first: A,
    second: U,
}

impl<A, U> AndThen<A, U> {
    pub fn new(first: A, second: U) -> Self {
        Self { first, second }
    }
}

#[async_trait]
impl<A, B, T> Validator<T> for AndThen<A, B>
where
    A: Validator<T> + Send + Sync,
    B: Validator<T, Error = A::Error> + Send + Sync,
    T: Sync,
{
    type Error = A::Error;

    async fn validate(&self, input: &T) -> Result<(), Self::Error> {
        self.first.validate(input).await?;
        self.second.validate(input).await?;
        Ok(())
    }
}
