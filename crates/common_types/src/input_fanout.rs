//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use crate::{NumPreshards, SubstateAddress};

/// The number of shard groups, beyond the first, that hold `addresses` under a layout of
/// `num_committees` committees over `num_preshards` shards.
///
/// Each shard group holding a transaction's inputs pledges them to every other group involved, so
/// a transaction whose `addresses` span `n` groups has its inputs carried to `n - 1` more groups than
/// one. Zero when every address falls in one group, or there are none.
pub fn input_fanout<I: IntoIterator<Item = SubstateAddress>>(
    num_preshards: NumPreshards,
    num_committees: u32,
    addresses: I,
) -> u32 {
    let groups = addresses
        .into_iter()
        .map(|address| address.to_shard_group(num_preshards, num_committees))
        .collect::<HashSet<_>>();
    u32::try_from(groups.len().saturating_sub(1)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An address whose first byte places it in a known shard: with 256 preshards, byte `b` lands in
    /// shard `b + 1`.
    fn address_in_byte(first_byte: u8) -> SubstateAddress {
        let mut bytes = [0u8; SubstateAddress::LENGTH];
        bytes[0] = first_byte;
        SubstateAddress::from_array(bytes)
    }

    #[test]
    fn addresses_in_one_group_have_no_fanout() {
        let addresses = [address_in_byte(0), address_in_byte(1), address_in_byte(127)];
        assert_eq!(input_fanout(NumPreshards::P256, 2, addresses), 0);
    }

    #[test]
    fn each_further_group_adds_one() {
        let addresses = [address_in_byte(0), address_in_byte(128)];
        assert_eq!(input_fanout(NumPreshards::P256, 2, addresses), 1);

        let addresses = [
            address_in_byte(0),
            address_in_byte(64),
            address_in_byte(128),
            address_in_byte(192),
        ];
        assert_eq!(input_fanout(NumPreshards::P256, 4, addresses), 3);
    }

    #[test]
    fn a_single_committee_has_no_fanout() {
        let addresses = [address_in_byte(0), address_in_byte(255)];
        assert_eq!(input_fanout(NumPreshards::P256, 1, addresses), 0);
    }

    #[test]
    fn no_addresses_have_no_fanout() {
        assert_eq!(input_fanout(NumPreshards::P256, 4, []), 0);
    }
}
