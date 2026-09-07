//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::types::{Amount, ComponentAddress, ResourceAddress, ResourceType, VaultId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VaultModel {
    pub account_address: ComponentAddress,
    pub id: VaultId,
    pub vault_version: u32,
    pub resource_address: ResourceAddress,
    pub resource_type: ResourceType,
    pub confidential_balance: Amount,
    pub revealed_balance: Amount,
    pub locked_revealed_balance: Amount,
    pub token_symbol: Option<String>,
    pub divisibility: u8,
}

impl VaultModel {
    /// The revealed balance not held by a wallet lock.
    ///
    /// A lock is held until its transaction resolves, while the revealed balance follows the chain, so a vault can
    /// be locked for more than it currently holds. Such a vault has nothing available to spend.
    pub fn available_revealed_balance(&self) -> Amount {
        self.revealed_balance.saturating_sub(self.locked_revealed_balance)
    }
}

#[derive(Debug, Clone)]
pub struct VaultBalance {
    pub account: ComponentAddress,
    pub confidential: Amount,
    pub revealed: Amount,
}

#[cfg(test)]
mod tests {
    use tari_template_lib::types::{ComponentAddress, ResourceType};

    use super::*;

    fn vault(revealed_balance: Amount, locked_revealed_balance: Amount) -> VaultModel {
        VaultModel {
            account_address: ComponentAddress::from_array([0u8; 32]),
            id: "vault_0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
            vault_version: 0,
            resource_address: "resource_0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
            resource_type: ResourceType::Stealth,
            confidential_balance: Amount::zero(),
            revealed_balance,
            locked_revealed_balance,
            token_symbol: None,
            divisibility: 6,
        }
    }

    #[test]
    fn available_revealed_balance_excludes_locked_funds() {
        assert_eq!(
            vault(Amount::new(100), Amount::new(40)).available_revealed_balance(),
            Amount::new(60)
        );
    }

    #[test]
    fn available_revealed_balance_is_zero_when_locked_exceeds_revealed() {
        assert_eq!(
            vault(Amount::new(123_004_298), Amount::new(123_009_691)).available_revealed_balance(),
            Amount::zero()
        );
    }
}
