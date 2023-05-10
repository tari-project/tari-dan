//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::net::TcpListener;

use tari_engine_types::substate::{SubstateAddress, SubstateDiff};
use tari_validator_node_cli::versioned_substate_address::VersionedSubstateAddress;

use crate::TariWorld;

pub fn get_os_assigned_ports() -> (u16, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port1 = listener.local_addr().unwrap().port();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port2 = listener.local_addr().unwrap().port();
    (port1, port2)
}

pub(crate) fn add_substate_addresses(world: &mut TariWorld, outputs_name: String, diff: &SubstateDiff) {
    let outputs = world.outputs.entry(outputs_name).or_default();
    let mut counters = [0usize, 0, 0, 0, 0, 0, 0, 0];
    for (addr, data) in diff.up_iter() {
        match addr {
            SubstateAddress::Component(_) => {
                let component = data.substate_value().component().unwrap();
                outputs.insert(
                    format!("components/{}", component.module_name),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[0] += 1;
            },
            SubstateAddress::Resource(_) => {
                outputs.insert(
                    format!("resources/{}", counters[1]),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[1] += 1;
            },
            SubstateAddress::Vault(_) => {
                outputs.insert(
                    format!("vaults/{}", counters[2]),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[2] += 1;
            },
            SubstateAddress::NonFungible(_) => {
                outputs.insert(
                    format!("nfts/{}", counters[3]),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[3] += 1;
            },
            SubstateAddress::UnclaimedConfidentialOutput(_) => {
                outputs.insert(
                    format!("layer_one_commitments/{}", counters[4]),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[4] += 1;
            },
            SubstateAddress::NonFungibleIndex(_) => {
                outputs.insert(
                    format!("nft_indexes/{}", counters[5]),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[5] += 1;
            },
            SubstateAddress::ExecuteResult(_) => {
                outputs.insert(
                    format!("execute_results/{}", counters[6]),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[6] += 1;
            },
        }
    }
}
