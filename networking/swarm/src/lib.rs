//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-4-Clause

mod behaviour;
pub mod config;
mod error;
pub mod inbound_limits;
mod protocol_version;

#[cfg(feature = "metrics")]
pub mod metrics;
pub mod protocol_ids;

pub use behaviour::*;
pub use config::Config;
pub use error::*;
pub use protocol_version::*;

pub type TariSwarm<TMsg> = libp2p::Swarm<TariNodeBehaviour<TMsg>>;

pub use libp2p::{identity, rendezvous, swarm};
pub use libp2p_messaging as messaging;
pub use libp2p_substream as substream;
