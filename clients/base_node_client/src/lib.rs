//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod error;
pub use error::BaseNodeClientError;

pub mod grpc;
pub mod types;

mod traits;
pub use traits::BaseNodeClient;
