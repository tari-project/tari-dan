//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Decoder, Encode, decode};
use tari_bor::adapters::bounded_vec;
use tari_template_abi::rust::{prelude::*, vec};

use super::{MAX_CONDITIONS_PER_CONJUNCTION, MAX_INCLUSION_PROOF_LEN, SpendWitness, StealthUnspentOutput};
use crate::{
    Amount,
    UtxoAddress,
    access_rules::MAX_ACCESS_RULE_NODES,
    crypto::{BalanceProofSignature, PedersenCommitmentBytes, RangeProofBytes, RistrettoPublicKeyBytes},
};

/// A revealed output together with the key authorised to take it.
///
/// The revealed funds materialise as a bucket in the executing workspace rather than as a stealth output, so unlike a
/// confidential output nothing in the statement says where they end up. `receiver` supplies that: the engine requires
/// the receiver's badge in the transaction's auth scope before it creates the bucket, so a statement lifted into
/// another transaction yields its revealed funds to nobody. The receiver authorises the reveal; where the bucket goes
/// afterwards is up to the instructions that take it.
#[derive(Debug, Clone, Copy, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct RevealedOutput {
    /// The amount of revealed funds to output. Must be positive.
    #[n(0)]
    pub amount: Amount,
    /// The public key whose badge must be in the transaction's auth scope for the revealed bucket to be created.
    #[n(1)]
    pub receiver: RistrettoPublicKeyBytes,
}

impl RevealedOutput {
    pub fn new(amount: Amount, receiver: RistrettoPublicKeyBytes) -> Self {
        Self { amount, receiver }
    }
}

/// A statement for stealth outputs. A statement must contain confidential outputs
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct StealthOutputsStatement {
    /// The stealth outputs that are to be created
    #[n(0)]
    #[cbor(decode_with = "decode_outputs")]
    pub outputs: Vec<StealthUnspentOutput>,
    /// The revealed funds to output, and the key authorised to take them. When set, a bucket containing the revealed
    /// stealth funds is created for `receiver`. `None` is the only encoding of "no revealed output": a zero amount is
    /// rejected, so the presence of this field and the presence of a revealed bucket are the same thing.
    #[n(1)]
    pub revealed_output: Option<RevealedOutput>,
    /// Bulletproof range proof for the output commitments proving that values are in the range
    /// [minimum_value_promise, 2^64)
    #[n(2)]
    pub agg_range_proof: RangeProofBytes,
}

impl StealthOutputsStatement {
    /// Create a new output statement with no stealth outputs, only a revealed amount taken by `receiver`.
    pub fn new_revealed_only(amount: Amount, receiver: RistrettoPublicKeyBytes) -> Self {
        Self {
            outputs: vec![],
            revealed_output: Some(RevealedOutput::new(amount, receiver)),
            agg_range_proof: RangeProofBytes::empty(),
        }
    }

    /// The revealed output amount, or zero when there is no revealed output.
    pub fn revealed_output_amount(&self) -> Amount {
        self.revealed_output.map_or(Amount::ZERO, |r| r.amount)
    }

    /// The key authorised to take the revealed output, if there is one.
    pub fn revealed_receiver(&self) -> Option<RistrettoPublicKeyBytes> {
        self.revealed_output.map(|r| r.receiver)
    }
}

/// A statement for stealth outputs to spend as inputs.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct StealthInput {
    /// The commitment of the unspent output being spent
    #[n(0)]
    pub commitment: PedersenCommitmentBytes,
    /// Selects which authorisation path the spender is exercising for this input (TIP-0006). Defaults to the key path.
    #[n(1)]
    #[cfg_attr(feature = "serde", serde(default))]
    #[cbor(default)]
    pub witness: SpendWitness,
}

impl StealthInput {
    /// A key-path spend of the output at `commitment`.
    pub fn new(commitment: PedersenCommitmentBytes) -> Self {
        Self {
            commitment,
            witness: SpendWitness::KeyPath,
        }
    }

    /// A spend of the output at `commitment` exercising `witness`.
    pub fn with_witness(commitment: PedersenCommitmentBytes, witness: SpendWitness) -> Self {
        Self { commitment, witness }
    }

    /// The nodes of the spend condition this input reveals, if any (see
    /// [`SpendCondition::num_nodes`](super::SpendCondition::num_nodes)).
    pub fn num_nodes(&self) -> usize {
        self.witness.as_script_path().map_or(0, |(leaf, _, _)| leaf.num_nodes())
    }
}

impl From<&UtxoAddress> for StealthInput {
    fn from(address: &UtxoAddress) -> Self {
        address.id().into_commitment_bytes().into()
    }
}

impl From<PedersenCommitmentBytes> for StealthInput {
    fn from(commitment: PedersenCommitmentBytes) -> Self {
        Self::new(commitment)
    }
}
impl From<&PedersenCommitmentBytes> for StealthInput {
    fn from(commitment: &PedersenCommitmentBytes) -> Self {
        (*commitment).into()
    }
}

/// A statement for stealth outputs. A statement must contain confidential outputs
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct StealthInputsStatement {
    /// The stealth inputs that are to be spent
    #[n(0)]
    #[cbor(decode_with = "decode_inputs")]
    pub inputs: Vec<StealthInput>,
    /// The total amount of revealed funds being spent.
    #[n(1)]
    pub revealed_amount: Amount,
}

impl StealthInputsStatement {
    pub fn new(inputs: Vec<StealthInput>, revealed_amount: Amount) -> Self {
        assert!(!revealed_amount.is_negative(), "Revealed amount must be non-negative");
        assert!(
            !inputs.is_empty() || !revealed_amount.is_zero(),
            "At least one input or a revealed amount must be provided"
        );
        Self {
            inputs,
            revealed_amount,
        }
    }

    /// Create a new input statement with no stealth inputs, only a revealed amount.
    pub fn new_revealed_only(amount: Amount) -> Self {
        Self::new(vec![], amount)
    }
}

/// A covenant sub-balance proof (TIP-0006 `AssertCovenantBalanced`) covering one partition of the transfer: the inputs
/// and outputs that share a `condition_root`. It proves the partition's committed value is conserved into outputs
/// carrying that root save for an exact cleartext `revealed_amount`, without exposing confidential values.
///
/// A claim is keyed by `partition_input_index` rather than by restating its condition root; the engine matches it to
/// its partition by that index. The partition's `condition_root` is bound into `signature`, so a claim cannot be
/// validated against the wrong partition.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct CovenantBalanceClaim {
    /// The index, within the transfer's spent inputs, of the first input belonging to this partition. Identifies the
    /// partition without restating its spend condition.
    #[n(0)]
    pub partition_input_index: u32,
    /// The exact net cleartext amount leaving the partition (zero for full conservation). Must be non-negative.
    #[n(1)]
    pub revealed_amount: Amount,
    /// Schnorr proof of knowledge of the partition's aggregate mask difference.
    #[n(2)]
    pub signature: BalanceProofSignature,
}

#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct StealthTransferStatement {
    #[n(0)]
    pub inputs_statement: StealthInputsStatement,
    #[n(1)]
    pub outputs_statement: StealthOutputsStatement,
    /// Balance proof that proves that no coins were created or destroyed during the transfer (assuming the range proof
    /// is valid). This may be None, if and only if, the transfer is revealed-only (i.e. no stealth inputs or outputs).
    #[n(2)]
    pub balance_proof: Option<BalanceProofSignature>,
    /// Covenant sub-balance proofs (TIP-0006), one per input partition (keyed by `condition_root`) whose predicate
    /// requires value conservation. Empty when no spent input gates on a covenant.
    #[n(3)]
    #[cfg_attr(feature = "serde", serde(default))]
    #[cbor(default, decode_with = "decode_covenant_claims")]
    pub covenant_claims: Vec<CovenantBalanceClaim>,
}

/// The most stealth inputs one [`StealthInputsStatement`] may spend. Decoding refuses more.
pub const MAX_STEALTH_INPUTS: usize = 1000;

/// The most stealth outputs one [`StealthOutputsStatement`] may create. Decoding refuses more.
pub const MAX_STEALTH_OUTPUTS: usize = 16;

// Each input decodes to a fixed-size value however few bytes encode it, and its spend condition counts toward one
// node bound shared by every input of the statement.
fn decode_inputs<'b, C>(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Vec<StealthInput>, decode::Error> {
    let mut num_nodes = 0;
    bounded_vec::decode_with_fn(d, ctx, MAX_STEALTH_INPUTS, |d, ctx| {
        let input = StealthInput::decode(d, ctx)?;
        num_nodes += input.num_nodes();
        if num_nodes > MAX_ACCESS_RULE_NODES {
            return Err(decode::Error::message(
                "stealth inputs hold more than MAX_ACCESS_RULE_NODES spend condition nodes",
            ));
        }
        Ok(input)
    })
}

fn decode_outputs<'b, C>(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Vec<StealthUnspentOutput>, decode::Error> {
    bounded_vec::decode(d, ctx, MAX_STEALTH_OUTPUTS)
}

// A transfer carries at most one claim per input partition, and so at most one per input.
fn decode_covenant_claims<'b, C>(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Vec<CovenantBalanceClaim>, decode::Error> {
    bounded_vec::decode(d, ctx, MAX_STEALTH_INPUTS)
}

impl StealthTransferStatement {
    pub fn revealed_only(input_amount: Amount, output_amount: Amount, receiver: RistrettoPublicKeyBytes) -> Self {
        Self {
            inputs_statement: StealthInputsStatement::new_revealed_only(input_amount),
            outputs_statement: StealthOutputsStatement::new_revealed_only(output_amount, receiver),
            balance_proof: None,
            covenant_claims: vec![],
        }
    }

    pub fn revealed_input_amount(&self) -> Amount {
        self.inputs_statement.revealed_amount
    }

    pub fn revealed_output_amount(&self) -> Amount {
        self.outputs_statement.revealed_output_amount()
    }

    /// The key authorised to take the revealed output, if there is one.
    pub fn revealed_receiver(&self) -> Option<RistrettoPublicKeyBytes> {
        self.outputs_statement.revealed_receiver()
    }

    pub fn stealth_outputs(&self) -> &[StealthUnspentOutput] {
        &self.outputs_statement.outputs
    }

    pub fn stealth_inputs(&self) -> &[StealthInput] {
        &self.inputs_statement.inputs
    }

    /// The nodes of every spend condition the inputs reveal (see
    /// [`SpendCondition::num_nodes`](super::SpendCondition::num_nodes)).
    pub fn num_nodes(&self) -> usize {
        self.stealth_inputs().iter().map(StealthInput::num_nodes).sum()
    }

    /// Whether every collection in this statement is within the bound its decode applies: the input, output and
    /// covenant claim counts, the atoms of each revealed spend condition, each inclusion proof's length, and the
    /// spend condition nodes of all inputs together.
    pub fn is_within_decode_limits(&self) -> bool {
        self.stealth_inputs().len() <= MAX_STEALTH_INPUTS &&
            self.stealth_outputs().len() <= MAX_STEALTH_OUTPUTS &&
            self.covenant_claims.len() <= MAX_STEALTH_INPUTS &&
            self.num_nodes() <= MAX_ACCESS_RULE_NODES &&
            self.stealth_inputs().iter().all(|input| {
                input.witness.as_script_path().is_none_or(|(leaf, proof, _)| {
                    leaf.conditions().len() <= MAX_CONDITIONS_PER_CONJUNCTION &&
                        proof.siblings.len() <= MAX_INCLUSION_PROOF_LEN
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Hash32,
        crypto::RangeProofBytes,
        stealth::{AtomicCondition, BuiltinPredicate, MerkleProof, SpendCondition},
    };

    fn statement(inputs: Vec<StealthInput>) -> StealthTransferStatement {
        StealthTransferStatement {
            inputs_statement: StealthInputsStatement {
                inputs,
                revealed_amount: Amount::ZERO,
            },
            outputs_statement: StealthOutputsStatement {
                outputs: vec![],
                revealed_output: None,
                agg_range_proof: RangeProofBytes::empty(),
            },
            balance_proof: None,
            covenant_claims: vec![],
        }
    }

    fn script_path_input(num_conditions: usize, proof_len: usize) -> StealthInput {
        let leaf =
            SpendCondition::all((0..num_conditions).map(|_| AtomicCondition::Builtin(BuiltinPredicate::AfterEpoch(0))));
        let proof = MerkleProof::new((0..proof_len).map(|_| Hash32::from_array([1; 32])).collect());
        StealthInput::with_witness(PedersenCommitmentBytes::zero(), SpendWitness::script_path(leaf, proof))
    }

    /// Decoding a statement and checking it against its decode limits agree.
    fn assert_decodes(statement: &StealthTransferStatement, expected: bool) {
        let bytes = tari_bor::encode(statement).unwrap();
        assert_eq!(tari_bor::decode::<StealthTransferStatement>(&bytes).is_ok(), expected);
        assert_eq!(statement.is_within_decode_limits(), expected);
    }

    #[test]
    fn statement_decodes_up_to_its_limits() {
        let inputs = |n| {
            (0..n)
                .map(|_| StealthInput::new(PedersenCommitmentBytes::zero()))
                .collect()
        };
        assert_decodes(&statement(inputs(MAX_STEALTH_INPUTS)), true);
        assert_decodes(&statement(inputs(MAX_STEALTH_INPUTS + 1)), false);

        assert_decodes(&statement(vec![script_path_input(1, MAX_INCLUSION_PROOF_LEN)]), true);
        assert_decodes(
            &statement(vec![script_path_input(1, MAX_INCLUSION_PROOF_LEN + 1)]),
            false,
        );
        assert_decodes(
            &statement(vec![script_path_input(MAX_CONDITIONS_PER_CONJUNCTION + 1, 0)]),
            false,
        );
    }

    #[test]
    fn spend_condition_nodes_are_bounded_across_inputs() {
        let per_input = MAX_CONDITIONS_PER_CONJUNCTION;
        let num_inputs = MAX_ACCESS_RULE_NODES / per_input;
        let at_limit = statement((0..num_inputs).map(|_| script_path_input(per_input, 0)).collect());
        assert_eq!(at_limit.num_nodes(), MAX_ACCESS_RULE_NODES);
        assert_decodes(&at_limit, true);

        let mut over_limit = at_limit;
        over_limit.inputs_statement.inputs.push(script_path_input(1, 0));
        assert_decodes(&over_limit, false);
    }
}
