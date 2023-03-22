//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use async_trait::async_trait;
use tari_dan_app_utilities::template_manager::TemplateManagerError;
use tari_dan_core::services::epoch_manager::EpochManagerError;
use thiserror::Error;
use tokio::sync::{mpsc::error::SendError, oneshot};

use crate::p2p::services::messaging::MessagingError;

mod handle;
pub use handle::{MempoolHandle, MempoolRequest};

mod initializer;
pub use initializer::spawn;

mod and_then;
pub use and_then::AndThen;

mod service;
mod validator;
pub use validator::{FeeTransactionValidator, TemplateExistsValidator};

#[derive(Error, Debug)]
pub enum MempoolError {
    #[error("Epoch Manager Error: {0}")]
    EpochManagerError(#[from] Box<EpochManagerError>),
    #[error("Broadcast failed: {0}")]
    BroadcastFailed(#[from] MessagingError),
    #[error("Invalid template address: {0}")]
    InvalidTemplateAddress(#[from] TemplateManagerError),
    #[error("Internal service request cancelled")]
    RequestCancelled,
    #[error("No fee instructions")]
    NoFeeInstructions,
}

impl From<SendError<MempoolRequest>> for MempoolError {
    fn from(_: SendError<MempoolRequest>) -> Self {
        Self::RequestCancelled
    }
}

impl From<oneshot::error::RecvError> for MempoolError {
    fn from(_: oneshot::error::RecvError) -> Self {
        Self::RequestCancelled
    }
}

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
