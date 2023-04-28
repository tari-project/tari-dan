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

use clap::{Args, Subcommand};
use tari_wallet_daemon_client::{types::WebRtcStartRequest, WalletDaemonClient};

#[derive(Debug, Subcommand, Clone)]
pub enum WebRtcSubcommand {
    #[clap(alias = "start")]
    Start(StartArgs),
}

#[derive(Debug, Args, Clone)]
pub struct StartArgs {
    pub signaling_server_token: String,
    pub webrtc_permissions_token: String,
}

impl WebRtcSubcommand {
    pub async fn handle(self, mut client: WalletDaemonClient) -> anyhow::Result<()> {
        #[allow(clippy::enum_glob_use)]
        use WebRtcSubcommand::*;
        match self {
            Start(args) => {
                let _resp = client
                    .webrtc_start(WebRtcStartRequest {
                        signaling_server_token: args.signaling_server_token,
                        permissions_token: args.webrtc_permissions_token,
                    })
                    .await?;
            },
        }
        Ok(())
    }
}
