//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Debug, Formatter};

pub trait Validator<T> {
    type Context;
    type Error;

    fn validate(&self, context: &Self::Context, input: &T) -> Result<(), Self::Error>;

    fn boxed(self) -> BoxedValidator<Self::Context, T, Self::Error>
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

    fn map_context<V, F>(self, f: F, validator: V) -> MapContext<Self, V, F>
    where
        V: Validator<T, Error = Self::Error>,
        F: Fn(&Self::Context) -> V::Context,
        Self: Sized,
    {
        MapContext::new(self, validator, f)
    }
}

pub struct BoxedValidator<C, T, E> {
    inner: Box<dyn Validator<T, Context = C, Error = E> + Send + Sync + 'static>,
}

impl<T: Send + Sync, C: Send + Sync, E> Validator<T> for BoxedValidator<C, T, E> {
    type Context = C;
    type Error = E;

    fn validate(&self, context: &Self::Context, input: &T) -> Result<(), Self::Error> {
        self.inner.validate(context, input)
    }
}

impl<C, T, E> Debug for BoxedValidator<C, T, E> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxedValidator")
            .field("inner", &"Box<dyn Validator....>")
            .finish()
    }
}

pub struct AndThen<A, B> {
    first: A,
    second: B,
}

impl<A, B> AndThen<A, B> {
    pub fn new(first: A, second: B) -> Self {
        Self { first, second }
    }
}

impl<A, B, T> Validator<T> for AndThen<A, B>
where
    A: Validator<T> + Send + Sync,
    B: Validator<T, Context = A::Context, Error = A::Error> + Send + Sync,
    T: Sync,
{
    type Context = A::Context;
    type Error = A::Error;

    fn validate(&self, context: &Self::Context, input: &T) -> Result<(), Self::Error> {
        self.first.validate(context, input)?;
        self.second.validate(context, input)?;
        Ok(())
    }
}

pub struct MapContext<A, B, F> {
    first: A,
    second: B,
    mapper: F,
}

impl<A, B, F> MapContext<A, B, F> {
    pub fn new(first: A, second: B, mapper: F) -> Self {
        Self { first, second, mapper }
    }
}

impl<A, B, T, F> Validator<T> for MapContext<A, B, F>
where
    A: Validator<T> + Send + Sync,
    B: Validator<T, Error = A::Error> + Send + Sync,
    F: Fn(&A::Context) -> B::Context,
    T: Sync,
{
    type Context = A::Context;
    type Error = A::Error;

    fn validate(&self, context: &Self::Context, input: &T) -> Result<(), Self::Error> {
        self.first.validate(context, input)?;
        self.second.validate(&(self.mapper)(context), input)?;
        Ok(())
    }
}
