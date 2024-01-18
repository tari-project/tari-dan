//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::{Epoch, SubstateAddress};

#[derive(Debug, Clone)]
pub enum EpochManagerEvent {
    EpochChanged(Epoch),
    ThisValidatorIsRegistered { epoch: Epoch, shard_key: SubstateAddress },
}
