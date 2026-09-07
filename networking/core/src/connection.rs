//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use libp2p::{PeerId, core::ConnectedPoint, swarm::ConnectionId};

#[derive(Debug, Clone)]
pub struct Connection {
    pub connection_id: ConnectionId,
    pub peer_id: PeerId,
    pub created_at: Instant,
    pub endpoint: ConnectedPoint,
    pub num_established: u32,
    pub num_concurrent_dial_errors: usize,
    pub established_in: Duration,
    pub ping_latency: Option<Duration>,
    /// Consecutive ping failures on this connection. Reset to zero by any successful ping.
    pub num_ping_failures: u32,
    pub user_agent: Option<Arc<String>>,
}

impl Connection {
    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }
}
