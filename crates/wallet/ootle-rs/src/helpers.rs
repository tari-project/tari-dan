//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_address::Network;

/// Returns the default indexer URL for the given network.
///
/// Currently configured for `LocalNet` (`http://localhost:12500`) and
/// `Esmeralda` (`https://esme-indexer-a.ootle.tari.com/`). Other networks are not yet configured.
pub fn default_indexer_url(network: Network) -> &'static str {
    match network {
        Network::MainNet => unimplemented!("MainNet indexer URL is not set"),
        Network::LocalNet => "http://localhost:12500",
        Network::StageNet => unimplemented!("StageNet indexer URL is not set"),
        Network::NextNet => unimplemented!("NextNet indexer URL is not set"),
        Network::Igor => unimplemented!("Igor indexer URL is not set"),
        Network::Esmeralda => "https://esme-indexer-a.ootle.tari.com/",
    }
}
