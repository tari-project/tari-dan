//  Copyright 2023. The Tari Project
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

use tari_ootle_storage::global::GlobalDb;
use tari_ootle_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_shutdown::ShutdownSignal;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tokio::task::JoinHandle;

use crate::{
    service::{EpochManagerHandle, config::EpochManagerConfig, epoch_manager_service::EpochManagerService},
    traits::EpochManagerSpec,
};

pub fn spawn_service<TSpec: EpochManagerSpec>(
    config: EpochManagerConfig,
    global_db: GlobalDb<SqliteGlobalDbAdapter<TSpec::Addr>>,
    node_public_key: RistrettoPublicKeyBytes,
    epoch_events: TSpec::EpochEventOracle,
    shutdown_signal: ShutdownSignal,
) -> (EpochManagerHandle<TSpec::Addr>, JoinHandle<anyhow::Result<()>>) {
    let (epoch_manager_handle, join_handle) =
        EpochManagerService::<TSpec>::spawn(config, global_db, epoch_events, node_public_key, shutdown_signal);
    (epoch_manager_handle, join_handle)
}
