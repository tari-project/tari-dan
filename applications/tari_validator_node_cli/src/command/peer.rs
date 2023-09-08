//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use clap::Subcommand;
use multiaddr::Multiaddr;
use tari_common_types::types::PublicKey;
use tari_crypto::tari_utilities::hex::Hex;
use tari_validator_node_client::{types::AddPeerRequest, ValidatorNodeClient};

#[derive(Debug, Subcommand, Clone)]
pub enum PeersSubcommand {
    Connect {
        public_key: String,
        addresses: Vec<Multiaddr>,
    },
}

impl PeersSubcommand {
    pub async fn handle(self, mut client: ValidatorNodeClient) -> anyhow::Result<()> {
        #[allow(clippy::enum_glob_use)]
        use PeersSubcommand::*;
        match self {
            Connect { public_key, addresses } => {
                client
                    .add_peer(AddPeerRequest {
                        public_key: PublicKey::from_hex(&public_key).map_err(anyhow::Error::msg)?,
                        addresses,
                        wait_for_dial: true,
                    })
                    .await?;
                println!("🫂 Peer connected");
            },
        }
        Ok(())
    }
}
