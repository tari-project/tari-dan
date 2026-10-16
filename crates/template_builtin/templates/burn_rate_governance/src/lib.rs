// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! The council's handle on the exhaust burn rate.
//!
//! The council is the component's owner rule: an m-of-n over the members' public identity badges,
//! which every transaction signer contributes to the authorization scope. Every method rule is
//! `DenyAll`, so the engine admits a call only when enough of the council signed it and no method
//! here checks a signer itself. Rotation and retirement replace that rule, which the engine gates on
//! the rule in force.
//!
//! The component is never read during transaction execution. Consensus decodes its state once per
//! epoch, when the end-of-epoch block that opens the next epoch is proposed, and the rate reaches
//! execution through the block header from there.

use tari_template_lib::{
    prelude::*,
    types::governance::{
        BurnRateChange,
        MAX_EXHAUST_BURN_RATE_BPS,
        MIN_BURN_RATE_ACTIVATION_LEAD_EPOCHS,
        council_owner_rule,
        no_council_owner_rule,
        prune_before,
        schedule_change,
    },
};

#[template]
mod template {
    use super::*;

    /// Wire-equivalent to `BurnRateGovernanceState`, which is what consensus decodes this
    /// component's state as.
    pub struct BurnRateGovernance {
        schedule: Vec<BurnRateChange>,
        retired_from: Option<u64>,
    }

    impl BurnRateGovernance {
        /// Schedules `rate_bps` to take effect from `activation_epoch`, replacing anything already
        /// scheduled at that epoch.
        ///
        /// Rescheduling an activation that has not happened yet is how the council aborts or amends
        /// one. An activation that has already happened cannot be rewritten: epochs that have run at
        /// a rate are not negotiable, and the lead time makes the boundary unambiguous.
        pub fn set_burn_rate(&mut self, rate_bps: u16, activation_epoch: u64) {
            assert!(
                rate_bps <= MAX_EXHAUST_BURN_RATE_BPS,
                "burn rate {rate_bps}bps is above the {MAX_EXHAUST_BURN_RATE_BPS}bps ceiling"
            );

            let current_epoch = Consensus::current_epoch();
            let earliest = current_epoch + MIN_BURN_RATE_ACTIVATION_LEAD_EPOCHS;
            assert!(
                activation_epoch >= earliest,
                "burn rate cannot activate at epoch {activation_epoch}: the earliest epoch a rate set in epoch \
                 {current_epoch} may activate at is {earliest}"
            );

            // Consensus reads this schedule only to open an epoch: the end-of-epoch block resolves the
            // next one, and a node starting consensus resolves the epoch it starts in. Neither opens an
            // epoch before the one this call executes in, so every entry they can still ask for survives.
            prune_before(&mut self.schedule, current_epoch.saturating_sub(1));
            schedule_change(&mut self.schedule, BurnRateChange {
                activation_epoch,
                rate_bps,
            });

            emit_event("set_burn_rate", metadata![
                "rate_bps" => rate_bps.to_string(),
                "activation_epoch" => activation_epoch.to_string(),
            ]);
        }

        /// Replaces the council with `council`, requiring `threshold` of them from the next
        /// transaction onwards. The members it drops no longer satisfy the owner rule.
        pub fn set_council(&mut self, threshold: u16, council: Vec<RistrettoPublicKeyBytes>) {
            ComponentManager::current().set_owner_rule(council_owner_rule(threshold, &council));

            emit_event("set_council", metadata![
                "threshold" => threshold.to_string(),
                "size" => council.len().to_string(),
            ]);
        }

        /// Gives up the component's say over the burn rate. The council is dismissed for good at
        /// once, since no caller satisfies the rule this leaves behind, but the rate hands back to the
        /// release-scheduled table only from the earliest epoch a rate set now could activate at.
        ///
        /// Until then the schedule governs as it did, which keeps a retirement from changing the rate
        /// of an epoch some shard group may already have resolved. From then on the rate comes from
        /// the table for as long as the network's source schedule names this component, so the
        /// network is never left without a rate.
        pub fn retire(&mut self) {
            let retired_from = Consensus::current_epoch() + MIN_BURN_RATE_ACTIVATION_LEAD_EPOCHS;
            self.retired_from = Some(retired_from);
            ComponentManager::current().set_owner_rule(no_council_owner_rule());

            emit_event("retire", metadata![
                "retired_from" => retired_from.to_string(),
            ]);
        }
    }
}
