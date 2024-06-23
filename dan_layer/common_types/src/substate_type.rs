//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub enum SubstateType {
    Component,
    Resource,
    Vault,
    UnclaimedConfidentialOutput,
    NonFungible,
    TransactionReceipt,
    FeeClaim,
}

impl SubstateType {
    pub fn as_prefix_str(&self) -> &str {
        match self {
            SubstateType::Component => "component",
            SubstateType::Resource => "resource",
            SubstateType::Vault => "vault",
            SubstateType::UnclaimedConfidentialOutput => "commitment",
            SubstateType::NonFungible => "nft",
            SubstateType::TransactionReceipt => "txreceipt",
            SubstateType::FeeClaim => "feeclaim",
        }
    }
}
