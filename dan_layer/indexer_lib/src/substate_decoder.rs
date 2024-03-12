//  Copyright 2023, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::collections::HashSet;

use log::*;
use tari_engine_types::{
    indexed_value::{IndexedValueError, IndexedWellKnownTypes},
    substate::{Substate, SubstateId, SubstateValue},
};

const LOG_TARGET: &str = "tari::dan::initializer::substate_decoder";

/// Recursively scan a substate for references to other substates
pub fn find_related_substates(substate: &Substate) -> Result<Vec<SubstateId>, IndexedValueError> {
    match substate.substate_value() {
        SubstateValue::Component(header) => {
            // Look inside the component state for substate references
            let value = IndexedWellKnownTypes::from_value(header.state())?;
            info!(target: LOG_TARGET, "Found indexed value: {:?}", &value);
            info!(
                target: LOG_TARGET,
                "Found {} substates in component state",
                value.referenced_substates().count()
            );
            Ok(value.referenced_substates().collect())
        },
        SubstateValue::NonFungible(nonfungible_container) => {
            let mut related_substates = vec![];

            if let Some(non_fungible) = nonfungible_container.contents() {
                let data = IndexedWellKnownTypes::from_value(non_fungible.data())?;
                let mutable_data = IndexedWellKnownTypes::from_value(non_fungible.mutable_data())?;
                related_substates.extend(
                    data.referenced_substates()
                        .chain(mutable_data.referenced_substates())
                        .collect::<HashSet<_>>(),
                );
                debug!(
                    target: LOG_TARGET,
                    "Found {} substates in non fungible state",
                    related_substates.len()
                );
            }
            Ok(related_substates)
        },
        SubstateValue::NonFungibleIndex(index) => {
            // by definition a non fungible index always holds a reference to a non fungible substate
            let substate_address = SubstateId::NonFungible(index.referenced_address().clone());
            Ok(vec![substate_address])
        },
        SubstateValue::Vault(vault) => Ok(vec![SubstateId::Resource(*vault.resource_address())]),
        // Other types of substates cannot hold references to other substates
        _ => Ok(vec![]),
    }
}
