// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! A network-wide multiplier on every fee charge, and the per-network schedule it is set by.
//!
//! The fee table prices each kind of work against the others. The scale moves the whole table at
//! once, so retuning what a transaction costs in TARI is one number in a release rather than an
//! edit to every rate.

use ootle_network::Network;

use crate::Epoch;

const BPS_PER_UNIT: u64 = 10_000;

/// A multiplier on every fee charge, in basis points: `10_000` charges the fee table as written.
///
/// Non-zero, so no scale makes a transaction free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FeeScale(u16);

impl FeeScale {
    pub const MAX: Self = Self(u16::MAX);
    pub const ONE: Self = Self(BPS_PER_UNIT as u16);

    /// Panics if `bps` is zero. The network schedules are const items, so for them that panic is a
    /// compile error.
    pub const fn new(bps: u16) -> Self {
        assert!(bps > 0, "fee scale must be non-zero");
        Self(bps)
    }

    /// `bps` as a scale, or `None` if it is zero.
    ///
    /// The checked counterpart of [`Self::new`], for a scale that arrives from outside this binary,
    /// such as a block header.
    pub const fn try_new(bps: u16) -> Option<Self> {
        if bps == 0 {
            return None;
        }
        Some(Self(bps))
    }

    pub const fn as_bps(self) -> u16 {
        self.0
    }

    pub const fn is_one(self) -> bool {
        self.0 as u64 == BPS_PER_UNIT
    }

    /// `amount` at this scale, rounded down.
    pub const fn apply(self, amount: u64) -> u64 {
        saturating_u64(amount as u128 * self.0 as u128 / BPS_PER_UNIT as u128)
    }

    /// `amount` at this scale, rounded up. An estimate that scales a sum with this is never below an
    /// engine that scales each of the sum's terms with [`Self::apply`].
    pub const fn apply_ceil(self, amount: u64) -> u64 {
        saturating_u64((amount as u128 * self.0 as u128).div_ceil(BPS_PER_UNIT as u128))
    }

    /// The largest unscaled amount whose [`Self::apply`] is at most `scaled`, i.e. what `scaled`
    /// microtari pays for in fee-table terms.
    pub const fn unapply(self, scaled: u64) -> u64 {
        saturating_u64(((scaled as u128 + 1) * BPS_PER_UNIT as u128 - 1) / self.0 as u128)
    }
}

impl Default for FeeScale {
    fn default() -> Self {
        Self::ONE
    }
}

const fn saturating_u64(value: u128) -> u64 {
    if value > u64::MAX as u128 {
        u64::MAX
    } else {
        value as u64
    }
}

/// The release-scheduled fee scale for each network.
pub struct FeeScaleSchedule;

impl FeeScaleSchedule {
    /// The scale schedule for `network`, ordered by activation epoch ascending. Entry at index 0 is
    /// the scale the network starts under.
    ///
    /// Networks run at independent epochs, so an entry is scheduled per network. An activation
    /// epoch must be far enough ahead that every validator runs the release carrying it before the
    /// epoch opens, since a validator still on the previous release charges the previous scale.
    const fn activations(network: Network) -> &'static [(Epoch, FeeScale)] {
        // Named consts rather than inline arrays: `FeeScale::new` asserts, so an array built inline
        // is a temporary rather than a promoted `'static`.
        const MAINNET: &[(Epoch, FeeScale)] = &[(Epoch(0), FeeScale::ONE)];
        const STAGENET: &[(Epoch, FeeScale)] = &[(Epoch(0), FeeScale::ONE)];
        const NEXTNET: &[(Epoch, FeeScale)] = &[(Epoch(0), FeeScale::ONE)];
        const IGOR: &[(Epoch, FeeScale)] = &[(Epoch(0), FeeScale::ONE)];
        const ESMERALDA: &[(Epoch, FeeScale)] = &[(Epoch(0), FeeScale::ONE)];
        const LOCALNET: &[(Epoch, FeeScale)] = &[(Epoch(0), FeeScale::ONE)];

        match network {
            Network::MainNet => MAINNET,
            Network::StageNet => STAGENET,
            Network::NextNet => NEXTNET,
            Network::Igor => IGOR,
            Network::Esmeralda => ESMERALDA,
            Network::LocalNet => LOCALNET,
        }
    }

    pub fn at(network: Network, epoch: Epoch) -> FeeScale {
        Self::activations(network)
            .iter()
            .rev()
            .find(|(at, _)| *at <= epoch)
            .map(|(_, scale)| *scale)
            .expect("scale schedule is never empty")
    }

    /// The newest scale this release schedules for `network`, whether or not it has activated.
    ///
    /// For a caller that prices a transaction without knowing the epoch it executes in. It is wrong
    /// only between a release and the activation epoch it schedules.
    pub fn latest(network: Network) -> FeeScale {
        Self::activations(network)
            .last()
            .map(|(_, scale)| *scale)
            .expect("scale schedule is never empty")
    }

    /// Every scale `network` is scheduled at.
    pub fn scheduled(network: Network) -> impl Iterator<Item = FeeScale> {
        Self::activations(network).iter().map(|(_, scale)| *scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_networks() -> Vec<Network> {
        (u8::MIN..=u8::MAX)
            .filter_map(|byte| Network::try_from(byte).ok())
            .collect()
    }

    #[test]
    fn every_network_starts_at_epoch_zero_and_stays_sorted() {
        for network in all_networks() {
            let activations = FeeScaleSchedule::activations(network);
            assert_eq!(activations[0].0, Epoch(0), "{network}");
            assert!(
                activations.windows(2).all(|pair| pair[0].0 <= pair[1].0),
                "{network} scale schedule must be sorted ascending by epoch"
            );
        }
    }

    #[test]
    fn far_future_resolves_to_the_latest_entry() {
        for network in all_networks() {
            assert_eq!(
                FeeScaleSchedule::at(network, Epoch(u64::MAX)),
                FeeScaleSchedule::latest(network),
                "{network}"
            );
        }
    }

    #[test]
    fn one_leaves_every_amount_unchanged() {
        for amount in [0, 1, 7, 9_999, 10_001, u64::MAX] {
            assert_eq!(FeeScale::ONE.apply(amount), amount);
            assert_eq!(FeeScale::ONE.apply_ceil(amount), amount);
            assert_eq!(FeeScale::ONE.unapply(amount), amount);
        }
    }

    #[test]
    fn apply_rounds_down_and_apply_ceil_rounds_up() {
        let scale = FeeScale::new(15_000);
        assert_eq!(scale.apply(3), 4);
        assert_eq!(scale.apply_ceil(3), 5);
        assert_eq!(scale.apply(4), 6);
        assert_eq!(scale.apply_ceil(4), 6);

        let scale = FeeScale::new(5_000);
        assert_eq!(scale.apply(1), 0);
        assert_eq!(scale.apply_ceil(1), 1);
    }

    #[test]
    fn scaling_saturates_rather_than_wrapping() {
        assert_eq!(FeeScale::MAX.apply(u64::MAX), u64::MAX);
        assert_eq!(FeeScale::MAX.apply_ceil(u64::MAX), u64::MAX);
        assert_eq!(FeeScale::new(1).unapply(u64::MAX), u64::MAX);
    }

    #[test]
    fn unapply_never_funds_more_than_was_paid() {
        for bps in [1, 3, 5_000, 10_000, 15_000, 33_333, u16::MAX] {
            let scale = FeeScale::new(bps);
            for paid in [0, 1, 2, 3, 99, 10_000, 123_456_789] {
                let funded = scale.unapply(paid);
                assert!(scale.apply(funded) <= paid, "{bps}bps, paid {paid}");
                assert!(scale.apply(funded + 1) > paid, "{bps}bps, paid {paid}");
            }
        }
    }

    #[test]
    fn zero_is_not_a_scale() {
        assert_eq!(FeeScale::try_new(0), None);
        assert_eq!(FeeScale::try_new(1), Some(FeeScale::new(1)));
    }
}
