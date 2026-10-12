//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Debug, Display, Formatter};

use borsh::BorshSerialize;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_consensus_types::{
    BlockId,
    LastExecuted,
    LastVoted,
    LeafBlock,
    LockedBlock,
    PcId,
    ProposalCertificate,
    ShardGroupAccumulatedData,
    SignedMessage,
    TcId,
    ToSignatureMessage,
};
use tari_crypto::tari_utilities::epoch_time::EpochTime;
use tari_engine_types::fees::ExhaustBurnRate;
use tari_ootle_common_types::{
    Epoch,
    ExtraData,
    ExtraFieldKey,
    NodeHeight,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    hashing,
};
use tari_ootle_transaction::Network;
use tari_sidechain::{BlockHeaderHashFields, BlockHeaderHashFieldsV1, BlockHeaderHashFieldsV2};
use tari_state_tree::{TreeHash, compute_merkle_root_for_hashes};
use tari_template_lib_types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

use super::{BlockCommands, BlockError, build_finalized_transaction_tree};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BlockHeader {
    /// "Cached" block ID/hash. This is computed from the contents of the block header.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[n(0)]
    id: BlockId,
    /// Network this block belongs to.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[n(1)]
    network: Network,
    /// Parent block ID.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[n(2)]
    parent: BlockId,
    /// The quorum certificate proposed in this block. Note that this QC justifies a previous block.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[n(3)]
    justify_id: PcId,
    /// Block height.
    #[n(4)]
    height: NodeHeight,
    /// Epoch this block belongs to.
    #[n(5)]
    epoch: Epoch,
    /// Shard group that created this block.
    #[n(6)]
    shard_group: ShardGroup,
    /// The public key of the proposer.
    #[n(7)]
    proposed_by: RistrettoPublicKeyBytes,
    /// The total leader fee for this block. This should match the sum of the leader fees in the block's body.
    #[n(8)]
    total_leader_fee: u64,
    /// A Merkle root hash committing to all state after this block has been applied.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(with = "ootle_serde::hex")]
    #[n(9)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    state_merkle_root: FixedHash,
    /// A Merkle root hash committing to commands in this block. It is zero if the block has no commands.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(with = "ootle_serde::hex")]
    #[n(10)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    command_merkle_root: FixedHash,
    /// Proposer signature that signs the Block ID
    #[n(11)]
    signature: Option<SchnorrSignatureBytes>,
    /// The Unix Epoch timestamp indicating the creation time of the block. Currently, this can be chosen arbitrarily
    /// and is only informational/used for metrics.
    #[n(12)]
    timestamp: u64,
    /// The epoch hash is a hash given by the epoch oracle. E.g. the base layer epoch oracle gives the first block hash
    /// of the epoch.
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(with = "ootle_serde::hex")]
    #[n(13)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    epoch_hash: FixedHash,
    /// Accumulated data for the shard group up to and including this block.
    #[n(14)]
    accumulated_data: ShardGroupAccumulatedData,
    /// Extra data to allow for potential future data to be provided as necessary without breaking changes.
    /// Currently, this is used to store the block's sidechain_id (if applicable).
    #[n(15)]
    extra_data: ExtraData,
    /// The protocol version this block was produced under, resolved from the network's activation schedule at
    /// [`Self::epoch`]. It makes the block self-describing: [`Self::calculate_hash`] and the L1 verifier in
    /// `tari_sidechain` both select the hash schema from this field rather than from the schedule.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    #[n(16)]
    protocol_version: ProtocolVersion,
    /// The id of the timeout certificate this block carries, or `None` when it carries none. It is part of the
    /// metadata hash and therefore of the signed block id, so a validity rule that reads the certificate
    /// (`check_justify_reaches_timeout_certificate`) reads data the proposer signed.
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[n(17)]
    timeout_certificate_id: Option<TcId>,
    /// A Merkle root over the outcome of each transaction this block finalizes, keyed by transaction id (see
    /// [`FinalizedTransactionLeaf`](super::FinalizedTransactionLeaf)). Present from protocol version 2, where the
    /// block id commits to it.
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[serde(default, with = "ootle_serde::hex::option")]
    #[n(18)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    transaction_merkle_root: Option<FixedHash>,
}

impl BlockHeader {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        network: Network,
        protocol_version: ProtocolVersion,
        parent: BlockId,
        justify_id: PcId,
        timeout_certificate_id: Option<TcId>,
        height: NodeHeight,
        epoch: Epoch,
        shard_group: ShardGroup,
        proposed_by: RistrettoPublicKeyBytes,
        state_merkle_root: FixedHash,
        commands: &BlockCommands,
        total_leader_fee: u64,
        signature: SchnorrSignatureBytes,
        timestamp: u64,
        epoch_hash: FixedHash,
        accumulated_data: ShardGroupAccumulatedData,
        extra_data: ExtraData,
    ) -> Result<Self, BlockError> {
        let mut header = Self::create_unsigned(
            network,
            protocol_version,
            parent,
            justify_id,
            timeout_certificate_id,
            height,
            epoch,
            shard_group,
            proposed_by,
            state_merkle_root,
            commands,
            total_leader_fee,
            timestamp,
            epoch_hash,
            accumulated_data,
            extra_data,
        )?;

        header.set_signature(signature);

        Ok(header)
    }

    /// A header for a proposal, to be signed with [`Self::set_signature`]. Its id is the id of the signed header, so
    /// [`Self::calculate_id`] matches [`Self::id`] only once the signature is set.
    #[allow(clippy::too_many_arguments)]
    pub fn create_unsigned(
        network: Network,
        protocol_version: ProtocolVersion,
        parent: BlockId,
        justify_id: PcId,
        timeout_certificate_id: Option<TcId>,
        height: NodeHeight,
        epoch: Epoch,
        shard_group: ShardGroup,
        proposed_by: RistrettoPublicKeyBytes,
        state_merkle_root: FixedHash,
        commands: &BlockCommands,
        total_leader_fee: u64,
        timestamp: u64,
        epoch_hash: FixedHash,
        accumulated_data: ShardGroupAccumulatedData,
        extra_data: ExtraData,
    ) -> Result<Self, BlockError> {
        let command_merkle_root = Self::compute_command_merkle_root(protocol_version, commands)?;
        let transaction_merkle_root = Self::compute_transaction_merkle_root(protocol_version, commands)?;
        let mut header = BlockHeader {
            id: BlockId::zero(),
            network,
            protocol_version,
            parent,
            justify_id,
            height,
            epoch,
            shard_group,
            proposed_by,
            state_merkle_root,
            command_merkle_root,
            transaction_merkle_root,
            total_leader_fee,
            signature: None,
            timestamp,
            epoch_hash,
            accumulated_data,
            extra_data,
            timeout_certificate_id,
        };
        // The header is signed after its id exists, so it hashes as the signed block it becomes.
        header.id = header.calculate_id_as(false);

        Ok(header)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn genesis(
        network: Network,
        protocol_version: ProtocolVersion,
        justify_id: PcId,
        epoch: Epoch,
        shard_group: ShardGroup,
        state_merkle_root: FixedHash,
        epoch_hash: FixedHash,
        accumulated_data: ShardGroupAccumulatedData,
        extra_data: ExtraData,
    ) -> Self {
        Self::create(
            network,
            protocol_version,
            BlockId::zero(),
            justify_id,
            None,
            NodeHeight::zero(),
            epoch,
            shard_group,
            RistrettoPublicKeyBytes::default(),
            state_merkle_root,
            &BlockCommands::empty(),
            0,
            SchnorrSignatureBytes::zero(),
            0,
            epoch_hash,
            accumulated_data,
            extra_data,
        )
        .expect("Infallible with empty commands")
    }

    /// This is the parent block for all genesis blocks. Its block ID is always zero.
    // TODO: do we need a zero block anymore?
    pub fn zero_block(network: Network, num_preshards: NumPreshards) -> Self {
        let shard_group = ShardGroup::all_shards(num_preshards);
        Self {
            network,
            protocol_version: ProtocolVersion::at(network, Epoch::zero()),
            id: BlockId::zero(),
            parent: BlockId::zero(),
            justify_id: ProposalCertificate::genesis(Epoch::zero(), ShardGroup::all_shards(num_preshards))
                .calculate_id(),
            height: NodeHeight::zero(),
            epoch: Epoch::zero(),
            shard_group,
            proposed_by: RistrettoPublicKeyBytes::default(),
            state_merkle_root: FixedHash::zero(),
            command_merkle_root: FixedHash::zero(),
            total_leader_fee: 0,
            // Not a dummy block
            signature: Some(SchnorrSignatureBytes::zero()),
            timestamp: EpochTime::now().as_u64(),
            epoch_hash: FixedHash::zero(),
            accumulated_data: ShardGroupAccumulatedData::default(),
            extra_data: ExtraData::new(),
            timeout_certificate_id: None,
            transaction_merkle_root: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dummy_block(
        network: Network,
        protocol_version: ProtocolVersion,
        parent: BlockId,
        proposed_by: RistrettoPublicKeyBytes,
        height: NodeHeight,
        justify_id: PcId,
        epoch: Epoch,
        shard_group: ShardGroup,
        parent_state_merkle_root: FixedHash,
        parent_timestamp: u64,
        parent_epoch_hash: FixedHash,
        parent_accumulated_data: ShardGroupAccumulatedData,
        parent_exhaust_burn_rate: ExhaustBurnRate,
    ) -> Self {
        let mut extra_data = ExtraData::new();
        extra_data.insert_bps(ExtraFieldKey::ExhaustBurnRate, parent_exhaust_burn_rate.as_bps());
        let mut block = Self {
            id: BlockId::zero(),
            network,
            protocol_version,
            parent,
            justify_id,
            height,
            epoch,
            shard_group,
            proposed_by,
            state_merkle_root: parent_state_merkle_root,
            command_merkle_root: BlockHeader::compute_command_merkle_root(protocol_version, &BlockCommands::empty())
                .expect("compute_command_merkle_root is infallible for empty commands"),
            total_leader_fee: 0,
            signature: None,
            timestamp: parent_timestamp,
            epoch_hash: parent_epoch_hash,
            accumulated_data: parent_accumulated_data,
            extra_data,
            timeout_certificate_id: None,
            transaction_merkle_root: BlockHeader::compute_transaction_merkle_root(
                protocol_version,
                &BlockCommands::empty(),
            )
            .expect("compute_transaction_merkle_root is infallible for empty commands"),
        };
        block.id = block.calculate_id();
        block
    }

    pub fn calculate_id(&self) -> BlockId {
        self.calculate_id_as(self.is_dummy())
    }

    fn calculate_id_as(&self, is_dummy: bool) -> BlockId {
        // Hash is created from the hash of the "body" and
        // then hashed with the parent, so that you can
        // create a merkle proof of a chain of blocks
        // ```pre
        // root
        // |\
        // |  block1
        // |\
        // |  block2
        // |
        // blockbody
        // ```

        let header_hash = self.calculate_hash_as(is_dummy);
        Self::calculate_block_id(&self.parent, &header_hash)
    }

    pub(crate) fn calculate_block_id(parent_id: &BlockId, header_hash: &FixedHash) -> BlockId {
        // The zero block is a special case. It has no parent and its ID is always zero.
        if *header_hash == FixedHash::zero() && parent_id.is_zero() {
            return BlockId::zero();
        }

        hashing::block_hasher()
            .chain(parent_id)
            .chain(header_hash)
            .finalize_into_array()
            .into()
    }

    /// The metadata hash is the one header field the base layer never recomputes: a commit proof carries it as an
    /// opaque value inside the header preimage that `tari_sidechain` hashes. Fields that only this network's
    /// validity rules read therefore commit here, which keeps the header preimage identical to the one the base
    /// layer verifies while still binding them into the block id.
    pub fn calculate_metadata_hash(&self) -> FixedHash {
        self.calculate_metadata_hash_as(self.is_dummy())
    }

    fn calculate_metadata_hash_as(&self, is_dummy: bool) -> FixedHash {
        let fields = MetadataHashFields::V1(MetadataHashFieldsV1 {
            total_leader_fee: self.total_leader_fee,
            timestamp: self.timestamp,
            extra_data: &self.extra_data,
            timeout_certificate_id: self.timeout_certificate_id.as_ref().map(TcId::hash),
            is_dummy,
        });
        hashing::block_metadata_hasher().chain(&fields).finalize().into()
    }

    pub fn calculate_hash(&self) -> FixedHash {
        self.calculate_hash_as(self.is_dummy())
    }

    fn calculate_hash_as(&self, is_dummy: bool) -> FixedHash {
        // This hash reduces proof sizes. A proof-of-commit only needs to include this hash and not
        // the data.
        let metadata_hash = self.calculate_metadata_hash_as(is_dummy);
        let accumulated_data = self.accumulated_data.into();

        let shard_group = tari_sidechain::ShardGroup {
            start: self.shard_group.start().as_u32(),
            end_inclusive: self.shard_group.end().as_u32(),
        };

        // This selection must stay identical to `tari_sidechain::SidechainBlockHeader::calculate_hash`, which is what
        // the base layer uses to verify a commit proof against the block ID a committee signed.
        let fields = match self.protocol_version {
            // `tari_sidechain` maps version 0 to the preimage that carries no version. A later version must commit
            // to itself through `BlockHeaderHashFields::V2`, so that two versions sharing a preimage shape still
            // produce distinct block IDs and the version a block claims cannot be altered without invalidating it.
            ProtocolVersion::V0 => BlockHeaderHashFields::V1(BlockHeaderHashFieldsV1 {
                network: self.network.as_byte(),
                justify_id: self.justify_id.hash(),
                height: self.height.as_u64(),
                epoch: self.epoch.as_u64(),
                epoch_hash: &self.epoch_hash,
                shard_group,
                proposed_by: self.proposed_by.as_bytes(),
                state_merkle_root: &self.state_merkle_root,
                command_merkle_root: &self.command_merkle_root,
                accumulated_data: &accumulated_data,
                metadata_hash: &metadata_hash,
            }),
            protocol_version @ (ProtocolVersion::V1 | ProtocolVersion::V2) => {
                BlockHeaderHashFields::V2(BlockHeaderHashFieldsV2 {
                    network: self.network.as_byte(),
                    protocol_version: protocol_version.as_u32(),
                    justify_id: self.justify_id.hash(),
                    height: self.height.as_u64(),
                    epoch: self.epoch.as_u64(),
                    epoch_hash: &self.epoch_hash,
                    shard_group,
                    proposed_by: self.proposed_by.as_bytes(),
                    state_merkle_root: &self.state_merkle_root,
                    command_merkle_root: &self.command_merkle_root,
                    accumulated_data: &accumulated_data,
                    transaction_merkle_root: self.transaction_merkle_root.as_ref(),
                    metadata_hash: &metadata_hash,
                })
            },
        };

        hashing::block_hasher().chain(&fields).finalize().into()
    }

    pub fn is_genesis(&self) -> bool {
        // TODO: simplify genesis - This check is used to skip some validations (e.g. signature). Are there some
        // malicious tricks with the other fields here? Ideally we'd simple do
        // `self == Self::genesis(self.epoch, self.shard_group)` however the previous epoch state hash makes that
        // difficult.
        self.height.is_zero() &&
            self.parent.is_zero() &&
            self.timestamp == 0 &&
            self.command_merkle_root.iter().all(|b| *b == 0) &&
            self.proposed_by.iter().all(|b| *b == 0) &&
            self.signature.is_none()
    }

    pub fn as_locked(&self) -> LockedBlock {
        LockedBlock {
            height: self.height,
            block_id: self.id,
            epoch: self.epoch,
        }
    }

    pub fn as_last_executed(&self) -> LastExecuted {
        LastExecuted {
            height: self.height,
            block_id: self.id,
            epoch: self.epoch,
        }
    }

    pub fn as_last_voted(&self) -> LastVoted {
        LastVoted {
            height: self.height,
            block_id: self.id,
            epoch: self.epoch,
        }
    }

    pub fn as_leaf(&self) -> LeafBlock {
        LeafBlock {
            height: self.height,
            block_id: self.id,
            epoch: self.epoch,
            shard_group: self.shard_group,
        }
    }

    pub fn id(&self) -> &BlockId {
        &self.id
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    pub fn parent(&self) -> &BlockId {
        &self.parent
    }

    pub fn justify_id(&self) -> &PcId {
        &self.justify_id
    }

    pub fn timeout_certificate_id(&self) -> Option<&TcId> {
        self.timeout_certificate_id.as_ref()
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn shard_group(&self) -> ShardGroup {
        self.shard_group
    }

    pub fn total_leader_fee(&self) -> u64 {
        self.total_leader_fee
    }

    pub fn total_transaction_fee(&self) -> u64 {
        self.total_leader_fee
    }

    pub fn proposed_by(&self) -> &RistrettoPublicKeyBytes {
        &self.proposed_by
    }

    pub fn state_merkle_root(&self) -> &FixedHash {
        &self.state_merkle_root
    }

    pub fn command_merkle_root(&self) -> &FixedHash {
        &self.command_merkle_root
    }

    pub fn transaction_merkle_root(&self) -> Option<&FixedHash> {
        self.transaction_merkle_root.as_ref()
    }

    pub fn is_dummy(&self) -> bool {
        self.signature.is_none()
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn signature(&self) -> Option<&SchnorrSignatureBytes> {
        self.signature.as_ref()
    }

    pub fn set_signature(&mut self, signature: SchnorrSignatureBytes) {
        self.signature = Some(signature);
    }

    pub fn accumulated_data(&self) -> &ShardGroupAccumulatedData {
        &self.accumulated_data
    }

    pub fn total_accumulated_exhaust_burn(&self) -> u128 {
        self.accumulated_data.total_exhaust_burn
    }

    pub fn epoch_hash(&self) -> &FixedHash {
        &self.epoch_hash
    }

    /// The exhaust burn rate in force for this block's epoch, or `None` if the header does not name
    /// one or names a value above the ceiling.
    ///
    /// `None` is what a validator rejects a proposal on. Everything downstream of validation reads
    /// [`Self::exhaust_burn_rate`] instead, because a header that reaches it has already been
    /// checked against the epoch's rate.
    pub fn try_exhaust_burn_rate(&self) -> Option<ExhaustBurnRate> {
        self.extra_data
            .get_bps(&ExtraFieldKey::ExhaustBurnRate)
            .and_then(ExhaustBurnRate::try_new)
    }

    /// The exhaust burn rate in force for this block's epoch.
    ///
    /// Every header a node accepts names a rate — `check_exhaust_burn_rate` rejects one that does
    /// not — so a header reaching this has one. A header that somehow does not reads as zero, which
    /// burns nothing.
    pub fn exhaust_burn_rate(&self) -> ExhaustBurnRate {
        self.try_exhaust_burn_rate().unwrap_or_default()
    }

    /// The exhaust burn rate the next epoch opens at, which only an end-of-epoch block names.
    pub fn next_epoch_exhaust_burn_rate(&self) -> Option<ExhaustBurnRate> {
        self.extra_data
            .get_bps(&ExtraFieldKey::NextEpochExhaustBurnRate)
            .and_then(ExhaustBurnRate::try_new)
    }

    pub fn extra_data(&self) -> &ExtraData {
        &self.extra_data
    }

    pub fn compute_command_merkle_root(
        protocol_version: ProtocolVersion,
        commands: &BlockCommands,
    ) -> Result<FixedHash, BlockError> {
        let hashes = commands
            .iter()
            .map(|cmd| TreeHash::from(cmd.hash(protocol_version).into_array()));
        let hash = compute_merkle_root_for_hashes(hashes).map_err(BlockError::StateTreeError)?;
        Ok(FixedHash::from(hash.into_array()))
    }

    /// The root over the transactions `commands` finalize, which a header carries from protocol version 2.
    pub fn compute_transaction_merkle_root(
        protocol_version: ProtocolVersion,
        commands: &BlockCommands,
    ) -> Result<Option<FixedHash>, BlockError> {
        match protocol_version {
            ProtocolVersion::V0 | ProtocolVersion::V1 => Ok(None),
            ProtocolVersion::V2 => {
                let root = build_finalized_transaction_tree(commands)?.root();
                Ok(Some(FixedHash::from(root.into_array())))
            },
        }
    }
}

impl Display for BlockHeader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.is_dummy() {
            write!(f, "Dummy")?;
        }
        write!(
            f,
            "[{}, {}, {}, {}->{}]",
            self.height(),
            self.epoch(),
            self.shard_group(),
            self.id(),
            self.parent()
        )
    }
}

// Used to sign the block
impl ToSignatureMessage for BlockHeader {
    fn to_signature_message(&self) -> FixedHash {
        *self.id.hash()
    }
}

impl SignedMessage for BlockHeader {
    fn signature(&self) -> &SchnorrSignatureBytes {
        // TODO: remove the Option for signature
        self.signature.as_ref().expect("BlockHeader not signed")
    }

    fn public_key(&self) -> &RistrettoPublicKeyBytes {
        &self.proposed_by
    }
}

#[derive(Debug, BorshSerialize)]
enum MetadataHashFields<'a> {
    V1(MetadataHashFieldsV1<'a>),
}

#[derive(Debug, BorshSerialize)]
struct MetadataHashFieldsV1<'a> {
    total_leader_fee: u64,
    timestamp: u64,
    extra_data: &'a ExtraData,
    timeout_certificate_id: Option<&'a FixedHash>,
    /// A dummy block and an empty proposal from the same leader can agree on every other field, and the signature
    /// that tells them apart signs the id. Committing to it here keeps them two blocks, so every node attributes
    /// the view the same way whichever of the two it holds.
    is_dummy: bool,
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::ProposalCertificate;

    use super::*;

    fn header(protocol_version: ProtocolVersion) -> BlockHeader {
        header_with_timeout_certificate(protocol_version, None)
    }

    fn header_with_timeout_certificate(
        protocol_version: ProtocolVersion,
        timeout_certificate_id: Option<TcId>,
    ) -> BlockHeader {
        let shard_group = ShardGroup::all_shards(NumPreshards::P64);
        BlockHeader::create(
            Network::LocalNet,
            protocol_version,
            BlockId::zero(),
            ProposalCertificate::genesis(Epoch(1), shard_group).calculate_id(),
            timeout_certificate_id,
            NodeHeight(2),
            Epoch(1),
            shard_group,
            RistrettoPublicKeyBytes::default(),
            FixedHash::zero(),
            &BlockCommands::empty(),
            1,
            SchnorrSignatureBytes::zero(),
            1234,
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::new(),
        )
        .unwrap()
    }

    #[test]
    fn a_header_without_a_transaction_merkle_root_encodes_as_before_the_field_existed() {
        let header = header_with_timeout_certificate(ProtocolVersion::V0, Some(TcId::from([4u8; 32])));
        let bytes = tari_bor::encode(&header).unwrap();

        // Fields 0 to 17, so a header stored before field 18 existed has this shape and decodes to the same header.
        let len = minicbor::Decoder::new(&bytes).array().unwrap();
        assert_eq!(len, Some(18));
        let decoded: BlockHeader = tari_bor::decode(&bytes).unwrap();
        assert_eq!(decoded.transaction_merkle_root(), None);
        assert_eq!(decoded.calculate_hash(), header.calculate_hash());
    }

    #[test]
    fn a_header_round_trips_its_protocol_version() {
        let header = header(ProtocolVersion::V0);
        let bytes = tari_bor::encode(&header).unwrap();
        let decoded: BlockHeader = tari_bor::decode(&bytes).unwrap();
        assert_eq!(decoded.protocol_version(), ProtocolVersion::V0);
        assert_eq!(decoded.calculate_hash(), header.calculate_hash());
    }

    #[test]
    fn a_header_round_trips_its_timeout_certificate_id() {
        let tc_id = TcId::from([7u8; 32]);
        let header = header_with_timeout_certificate(ProtocolVersion::V0, Some(tc_id));
        let decoded: BlockHeader = tari_bor::decode(&tari_bor::encode(&header).unwrap()).unwrap();
        assert_eq!(decoded.timeout_certificate_id(), Some(&tc_id));
        assert_eq!(decoded.id(), header.id());
    }

    #[test]
    fn the_timeout_certificate_id_is_in_the_block_id() {
        let protocol_version = ProtocolVersion::V0;
        let without = header(protocol_version);
        let with = header_with_timeout_certificate(protocol_version, Some(TcId::from([7u8; 32])));
        let with_other = header_with_timeout_certificate(protocol_version, Some(TcId::from([8u8; 32])));
        assert_ne!(without.calculate_metadata_hash(), with.calculate_metadata_hash());
        assert_ne!(with.calculate_metadata_hash(), with_other.calculate_metadata_hash());
        assert_ne!(without.id(), with.id());
        assert_ne!(with.id(), with_other.id());
    }

    /// An empty proposal made in the same second as its parent matches the dummy block for its view in every
    /// field but the signature.
    #[test]
    fn a_dummy_block_never_shares_an_id_with_an_empty_proposal() {
        let protocol_version = ProtocolVersion::V0;
        let shard_group = ShardGroup::all_shards(NumPreshards::P64);
        let parent = BlockId::from([1u8; 32]);
        let justify_id = ProposalCertificate::genesis(Epoch(1), shard_group).calculate_id();
        let proposed_by = RistrettoPublicKeyBytes::default();
        let accumulated_data = ShardGroupAccumulatedData::default();
        let parent_timestamp = 1234;
        let exhaust_burn_rate = ExhaustBurnRate::new(500);

        let dummy = BlockHeader::dummy_block(
            Network::LocalNet,
            protocol_version,
            parent,
            proposed_by,
            NodeHeight(2),
            justify_id,
            Epoch(1),
            shard_group,
            FixedHash::zero(),
            parent_timestamp,
            FixedHash::zero(),
            accumulated_data,
            exhaust_burn_rate,
        );
        let mut extra_data = ExtraData::new();
        extra_data.insert_bps(ExtraFieldKey::ExhaustBurnRate, exhaust_burn_rate.as_bps());
        let proposal = BlockHeader::create(
            Network::LocalNet,
            protocol_version,
            parent,
            justify_id,
            None,
            NodeHeight(2),
            Epoch(1),
            shard_group,
            proposed_by,
            FixedHash::zero(),
            &BlockCommands::empty(),
            0,
            SchnorrSignatureBytes::zero(),
            parent_timestamp,
            FixedHash::zero(),
            accumulated_data,
            extra_data,
        )
        .unwrap();

        assert_ne!(dummy.id(), proposal.id());
        assert_eq!(dummy.calculate_id(), *dummy.id());
        assert_eq!(proposal.calculate_id(), *proposal.id());
    }
}
