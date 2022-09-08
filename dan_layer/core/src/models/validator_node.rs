use tari_comms::types::CommsPublicKey;

pub struct ValidatorNode {
    pub shard_key: [u8; 32],
    pub public_key: CommsPublicKey,
}
