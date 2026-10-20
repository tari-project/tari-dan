//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use blake2::{Blake2b, digest::consts::U32};
use ootle_network::Network;
use tari_hashing::BlocksHashDomain;
use tari_node_components::blocks::BlockHeader;
use tari_sidechain::FixedHash;
use tari_transaction_components::consensus::DomainSeparatedConsensusHasher;

// TODO: we duplicate the hashing here because of the CURRENT_NETWORK global that is used in L1. This should not be used
// on L2 to avoid the associated problems with this approach.

pub(super) fn hash_header(network: Network, header: &BlockHeader) -> FixedHash {
    DomainSeparatedConsensusHasher::<BlocksHashDomain, Blake2b<U32>>::new_with_network(
        "block_header",
        network.as_byte(),
    )
    .chain(&mining_hash(network, header))
    .chain(&header.pow)
    .chain(&header.nonce)
    .finalize()
    .into()
}

fn mining_hash(network: Network, header: &BlockHeader) -> FixedHash {
    DomainSeparatedConsensusHasher::<BlocksHashDomain, Blake2b<U32>>::new_with_network(
        "block_header",
        network.as_byte(),
    )
    .chain(&header.version)
    .chain(&header.height)
    .chain(&header.prev_hash)
    .chain(&header.timestamp)
    .chain(&header.input_mr)
    .chain(&header.output_mr)
    .chain(&header.output_smt_size)
    .chain(&header.block_output_mr)
    .chain(&header.kernel_mr)
    .chain(&header.kernel_mmr_size)
    .chain(&header.total_kernel_offset)
    .chain(&header.total_script_offset)
    .chain(&header.validator_node_mr)
    .chain(&header.validator_node_size)
    .finalize()
    .into()
}
