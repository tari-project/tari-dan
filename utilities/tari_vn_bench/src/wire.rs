//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Encoded sizes of the things a validator sends, receives and stores.
//!
//! Bandwidth and disk requirements are both products of a rate the protocol fixes and a size the
//! implementation fixes. The rate is knowable from the consensus constants; the size is not — it
//! depends on how much `Evidence` a command carries, and evidence holds a full [`SubstateId`] for
//! every input and output the transaction touches, in every shard group it touches. That is the
//! dominant term in a block and it cannot be estimated to better than a factor of two by reading
//! the types.
//!
//! So it is measured here rather than guessed. Nothing in this module needs a network, a database
//! or even an engine: it constructs the real structures and encodes them with the real codec, which
//! makes the answer deterministic and identical on every machine. That is also why these figures
//! are reported once and are not part of the per-host grading — they describe the protocol, not the
//! box.
//!
//! **Codec caveat.** Sizes here are CBOR, which is exactly what the state store persists, so the
//! disk figures are direct. Consensus messages go over the wire as protobuf, which encodes the same
//! data slightly more tightly (integer field tags rather than map keys). Treat the bandwidth
//! figures derived from these as a modest over-estimate rather than a floor.

use serde::{Deserialize, Serialize};
use tari_consensus_types::Decision;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{NumPreshards, VersionedSubstateId};
use tari_ootle_storage::consensus_models::{Command, Evidence, LeaderFee, TransactionAtom};
use tari_ootle_transaction::{Transaction, args};
use tari_template_lib::types::{
    ComponentAddress,
    ObjectKey,
    TransactionReceiptAddress,
    VaultId,
    constants::TARI_TOKEN,
};

/// Committee counts the evidence measurement is taken at.
///
/// Evidence is keyed by shard group, so a transaction whose substates land in more groups carries
/// more of it. One committee is the whole network in a single shard group — the floor. The larger
/// value shows how the per-command cost grows as the network shards, which is the number that
/// matters for sizing a mainnet that is expected to split.
const COMMITTEE_COUNTS: [u32; 3] = [1, 4, 16];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMeasurement {
    /// A canonical transfer as a wallet submits it and gossip carries it. This is what mempool
    /// traffic is made of.
    pub transaction_bytes: usize,
    /// One block command, at each of [`COMMITTEE_COUNTS`], as `(num_committees, bytes)`. Blocks
    /// carry these rather than transaction payloads, so this — not `transaction_bytes` — is what
    /// block propagation costs.
    pub command_bytes_by_committees: Vec<(u32, usize)>,
    /// Commands in a maximum-size block, from `max_commands_in_block`.
    pub max_commands_in_block: usize,
    /// Command payload of a maximum-size block at the largest measured committee count. Excludes
    /// the header and certificates, which are a fixed cost of a few KiB against this.
    pub max_block_command_bytes: usize,
}

pub fn measure(max_commands_in_block: usize) -> anyhow::Result<WireMeasurement> {
    let transaction = canonical_transfer();
    let transaction_bytes = tari_bor::encode(&transaction)?.len();

    let command_bytes_by_committees = COMMITTEE_COUNTS
        .into_iter()
        .map(|num_committees| {
            let command = command_for(&transaction, num_committees);
            Ok((num_committees, tari_bor::encode(&command)?.len()))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    // Size a full block at the widest sharding measured: evidence grows with committee count, so
    // the largest is the one a capacity plan has to survive.
    let worst_command_bytes = command_bytes_by_committees
        .iter()
        .map(|(_, bytes)| *bytes)
        .max()
        .expect("COMMITTEE_COUNTS is not empty");

    Ok(WireMeasurement {
        transaction_bytes,
        command_bytes_by_committees,
        max_commands_in_block,
        max_block_command_bytes: worst_command_bytes * max_commands_in_block,
    })
}

/// The same shape [`crate::execution`] benchmarks: a fee payment plus a one-token account-to-account
/// transfer, with both accounts declared as inputs. Built without a harness — only the encoded form
/// is wanted, so nothing needs to execute.
fn canonical_transfer() -> Transaction {
    let key = RistrettoSecretKey::from(42u64);
    let sender = ComponentAddress::from_array([0x22; 32]);
    let receiver = ComponentAddress::from_array([0xdd; 32]);

    Transaction::builder_localnet(tari_ootle_transaction::Epoch(1))
        .with_unversioned_inputs([sender, receiver])
        .pay_fee_from_component(sender, 1_000_000u64)
        .call_method(sender, "withdraw", args![TARI_TOKEN, 1])
        .put_last_instruction_output_on_workspace("transferred")
        .call_method(receiver, "deposit", args![Workspace("transferred")])
        .build_and_seal(&key)
}

/// Wraps the transaction in the command a block would carry it as, with the evidence a validator
/// would have derived for it.
///
/// Evidence is the term that decides a block's size, and it holds a full [`SubstateId`] for every
/// input *and every output*, in every shard group the transaction touches. So the outputs a
/// transfer really produces are included — two updated vaults and a transaction receipt — because
/// leaving them out understates a command by roughly the share of substates they represent.
///
/// The substate addresses are spread across the address space on purpose. Shard group is derived
/// from the address, so addresses clustered together land in one group no matter how many
/// committees exist, and the measurement would report that sharding is free. Spreading them lets
/// the sweep over `num_committees` show what it is there to show.
fn command_for(transaction: &Transaction, num_committees: u32) -> Command {
    let outputs = output_substates();

    let evidence = Evidence::from_inputs_and_outputs(
        NumPreshards::current(),
        num_committees,
        transaction.all_inputs_iter(),
        outputs,
    );

    Command::LocalOnly(TransactionAtom {
        id: transaction.calculate_id(),
        decision: Decision::Commit,
        evidence,
        transaction_fee: 1_000,
        leader_fee: Some(LeaderFee {
            fee: 100,
            exhaust_burn: 5,
        }),
    })
}

/// What a committed transfer ups: the sender's and receiver's vaults, and the transaction receipt.
fn output_substates() -> Vec<VersionedSubstateId> {
    // Distinct leading bytes so the addresses fall in different preshards, and therefore in
    // different shard groups once the network has more than one committee.
    vec![
        VersionedSubstateId::new(SubstateId::Vault(VaultId::new(ObjectKey::from_array([0x11; 32]))), 1),
        VersionedSubstateId::new(SubstateId::Vault(VaultId::new(ObjectKey::from_array([0x88; 32]))), 1),
        VersionedSubstateId::new(
            SubstateId::TransactionReceipt(TransactionReceiptAddress::from_array([0xcc; 32])),
            0,
        ),
    ]
}
