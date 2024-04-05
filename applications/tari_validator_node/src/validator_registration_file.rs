use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_core::transactions::transaction_components::ValidatorNodeSignature;

#[derive(Serialize, Deserialize)]
pub struct ValidatorRegistrationFile {
    pub signature: ValidatorNodeSignature,
    pub public_key: PublicKey,
    pub claim_public_key: PublicKey,
}