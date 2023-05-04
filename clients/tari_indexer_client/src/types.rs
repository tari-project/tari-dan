//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{serde_with as serde_tools, PayloadId};
use tari_engine_types::{
    commit_result::ExecuteResult,
    substate::{Substate, SubstateAddress},
};
use tari_transaction::Transaction;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubstateRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateAddress,
    pub version: Option<u32>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSubstateResponse {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateAddress,
    pub version: u32,
    pub substate: Substate,
    #[serde(with = "serde_tools::hex")]
    pub created_by_transaction: FixedHash,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectSubstateRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateAddress,
    pub version: Option<u32>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectSubstateResponse {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateAddress,
    pub version: u32,
    pub substate_contents: serde_json::Value,
    #[serde(with = "serde_tools::hex")]
    pub created_by_transaction: FixedHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionRequest {
    pub transaction: Transaction,
    pub is_dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionResponse {
    #[serde(with = "serde_tools::hex")]
    pub transaction_hash: FixedHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionResultRequest {
    pub transaction_hash: PayloadId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTransactionResultResponse {
    pub execution_result: Option<ExecuteResult>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddAddressRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateAddress,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteAddressRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateAddress,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetNonFungibleCountRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateAddress,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetNonFungiblesRequest {
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateAddress,
    pub start_index: u64,
    pub end_index: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetNonFungiblesResponse {
    pub non_fungibles: Vec<NonFungibleSubstate>,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct NonFungibleSubstate {
    pub index: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub address: SubstateAddress,
    pub substate: Substate,
}
