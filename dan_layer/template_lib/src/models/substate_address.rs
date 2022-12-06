//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};

use crate::models::{ComponentAddress, ResourceAddress, VaultId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
pub enum SubstateAddress {
    Component(ComponentAddress),
    Resource(ResourceAddress),
    Vault(VaultId),
}
