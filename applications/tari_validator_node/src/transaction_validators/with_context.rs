//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use crate::validator::Validator;

#[derive(Debug, Default)]
pub struct WithContext<C, T, E>(PhantomData<(C, T, E)>);

impl<C, T, E> WithContext<C, T, E> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<C, T, E> Validator<T> for WithContext<C, T, E> {
    type Context = C;
    type Error = E;

    fn validate(&self, _context: &C, _input: &T) -> Result<(), Self::Error> {
        Ok(())
    }
}
