//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_ootle_common_types::Epoch;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{StateStoreReadTransaction, StorageError};

#[derive(Debug, Clone, Copy)]
pub struct ValidatorStatsUpdate<'a> {
    public_key: &'a RistrettoPublicKeyBytes,

    /// None = no change, Some(n) = inc failure by n, Some(0) = clear failures
    missed_proposal_change: Option<i64>,
    participation_shares_increment: u64,
    max_missed_proposal_count: u64,
}

impl<'a> ValidatorStatsUpdate<'a> {
    pub fn new(public_key: &'a RistrettoPublicKeyBytes) -> Self {
        Self {
            public_key,
            missed_proposal_change: None,
            participation_shares_increment: 0,
            max_missed_proposal_count: 5,
        }
    }

    pub fn public_key(&self) -> &RistrettoPublicKeyBytes {
        self.public_key
    }

    pub fn missed_proposal_change(&self) -> Option<i64> {
        self.missed_proposal_change
    }

    pub fn participation_shares_increment(&self) -> u64 {
        self.participation_shares_increment
    }

    pub fn add_missed_proposal(mut self) -> Self {
        self.missed_proposal_change = Some(1);
        self
    }

    pub fn decrement_missed_proposal(mut self) -> Self {
        self.missed_proposal_change = Some(-1);
        self
    }

    /// Sets a cap for the missed proposal count.
    pub fn set_max_missed_proposals_cap(mut self, n: u64) -> Self {
        self.max_missed_proposal_count = n;
        self
    }

    pub fn max_total_missed_proposals(&self) -> i64 {
        i64::try_from(self.max_missed_proposal_count).unwrap_or(i64::MAX)
    }

    pub fn reset_missed_proposals(mut self) -> Self {
        self.missed_proposal_change = Some(0);
        self
    }

    pub fn increment_participation_share(mut self) -> Self {
        self.participation_shares_increment = 1;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct ValidatorConsensusStats {
    #[n(0)]
    pub missed_proposals: u64,
    #[n(1)]
    pub participation_shares: u64,
}

impl ValidatorConsensusStats {
    pub fn get_by_public_key<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        epoch: Epoch,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<Self, StorageError> {
        tx.validator_epoch_stats_get(epoch, public_key)
    }
}
