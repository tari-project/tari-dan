//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use indexmap::IndexSet;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::{indexed_value::IndexedValueError, substate::SubstateId};
use tari_ootle_common_types::{Epoch, InputDeclaration, declare_inputs};
use tari_template_lib_types::{ComponentAddress, crypto::RistrettoPublicKeyBytes};

use crate::{
    BlobIndex,
    Blobs,
    Instruction,
    IntoSigned,
    MainSigner,
    Signable,
    TransactionSignature,
    UnsealedTransactionV1,
    UnsignedTransactionV1,
    blobs::{Blob, BlobIndexOverflow},
    unsealed::UnsealedTransaction,
};

#[derive(Debug, Clone, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum UnsignedTransaction {
    #[n(0)]
    V1(#[n(0)] UnsignedTransactionV1),
}

impl UnsignedTransaction {
    pub fn new<N: Into<u8>>(network: N, max_epoch: Epoch) -> Self {
        Self::V1(UnsignedTransactionV1::new_default(network, max_epoch))
    }

    pub fn then<F: FnOnce(Self) -> Self>(self, f: F) -> Self {
        f(self)
    }

    pub fn schema_version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
        }
    }

    pub fn set_network<N: Into<u8>>(&mut self, network: N) -> &mut Self {
        match self {
            Self::V1(tx) => tx.set_network(network),
        };
        self
    }

    pub fn network(&self) -> u8 {
        match self {
            Self::V1(tx) => tx.network,
        }
    }

    pub fn is_dry_run(&self) -> bool {
        match self {
            Self::V1(tx) => tx.dry_run,
        }
    }

    pub(crate) fn set_dry_run(&mut self, dry_run: bool) -> &mut Self {
        match self {
            Self::V1(tx) => tx.set_dry_run(dry_run),
        };
        self
    }

    pub fn main_signer(&self) -> &MainSigner {
        match self {
            Self::V1(tx) => &tx.main_signer,
        }
    }

    /// Sets the transaction's main signer, which also decides whether the seal signature authorises the
    /// transaction.
    ///
    /// The field defaults to [`MainSigner::SealSigner`] and is part of the signing domain; sealing never changes
    /// it, so the value set here is the one every signature commits to and the engine enforces.
    pub fn set_main_signer(&mut self, main_signer: MainSigner) -> &mut Self {
        match self {
            Self::V1(tx) => tx.main_signer = main_signer,
        }
        self
    }

    pub fn fee_instructions(&self) -> &[Instruction] {
        match self {
            Self::V1(tx) => tx.fee_instructions(),
        }
    }

    pub(crate) fn fee_instructions_mut(&mut self) -> &mut Vec<Instruction> {
        match self {
            Self::V1(tx) => &mut tx.fee_instructions,
        }
    }

    pub fn instructions(&self) -> &[Instruction] {
        match self {
            Self::V1(tx) => tx.instructions(),
        }
    }

    pub(crate) fn instructions_mut(&mut self) -> &mut Vec<Instruction> {
        match self {
            Self::V1(tx) => &mut tx.instructions,
        }
    }

    pub fn into_instructions(self) -> Vec<Instruction> {
        match self {
            Self::V1(tx) => tx.instructions,
        }
    }

    pub fn inputs(&self) -> &IndexSet<InputDeclaration> {
        match self {
            Self::V1(tx) => tx.inputs(),
        }
    }

    pub fn add_input(&mut self, input: InputDeclaration) -> &mut Self {
        match self {
            Self::V1(tx) => {
                tx.add_input(input);
            },
        }
        self
    }

    pub fn min_epoch(&self) -> Option<Epoch> {
        match self {
            Self::V1(tx) => tx.min_epoch(),
        }
    }

    pub fn max_epoch(&self) -> Epoch {
        match self {
            Self::V1(tx) => tx.max_epoch(),
        }
    }

    pub fn as_referenced_components(&self) -> impl Iterator<Item = &ComponentAddress> + '_ {
        match self {
            Self::V1(tx) => tx.as_referenced_components(),
        }
    }

    /// Returns all substates addresses referenced by this transaction
    pub fn to_referenced_substates(&self) -> Result<HashSet<SubstateId>, IndexedValueError> {
        match self {
            Self::V1(tx) => tx.to_referenced_substates(),
        }
    }

    pub fn has_inputs_without_version(&self) -> bool {
        self.inputs().iter().any(|i| i.version().is_none())
    }

    pub fn set_min_epoch(&mut self, min_epoch: Option<Epoch>) -> &mut Self {
        match self {
            Self::V1(tx) => tx.min_epoch = min_epoch,
        }
        self
    }

    pub fn set_max_epoch(&mut self, max_epoch: Epoch) -> &mut Self {
        match self {
            Self::V1(tx) => tx.max_epoch = max_epoch,
        }
        self
    }

    pub fn nonce(&self) -> u64 {
        match self {
            Self::V1(tx) => tx.nonce,
        }
    }

    pub fn set_nonce(&mut self, nonce: u64) -> &mut Self {
        match self {
            Self::V1(tx) => tx.nonce = nonce,
        }
        self
    }

    pub fn with_inputs<I: IntoIterator<Item = InputDeclaration>>(mut self, inputs: I) -> Self {
        declare_inputs(self.inputs_mut(), inputs);
        self
    }

    /// Declares as a read each write-declared input for which `is_read` returns true, keeping the
    /// declarations and their order. A read declaration is left a read.
    pub fn narrow_to_reads<F: FnMut(&InputDeclaration) -> bool>(mut self, mut is_read: F) -> Self {
        let inputs = std::mem::take(self.inputs_mut());
        *self.inputs_mut() = inputs
            .into_iter()
            .map(|decl| {
                let narrow = decl.is_write() && is_read(&decl);
                if narrow { decl.with_is_write(false) } else { decl }
            })
            .collect();
        self
    }

    pub fn add_signer(
        mut self,
        seal_signer: &RistrettoPublicKeyBytes,
        key: &RistrettoSecretKey,
    ) -> UnsealedTransaction {
        match &mut self {
            Self::V1(tx) => UnsealedTransactionV1::new(tx.clone(), vec![])
                .add_signer(seal_signer, key)
                .into(),
        }
    }

    pub(crate) fn add_single_signature(mut self, signature: TransactionSignature) -> UnsealedTransaction {
        match &mut self {
            Self::V1(tx) => UnsealedTransactionV1::new(tx.clone(), vec![signature]).into(),
        }
    }

    pub fn with_signatures(self, signatures: Vec<TransactionSignature>) -> UnsealedTransaction {
        match self {
            UnsignedTransaction::V1(tx) => UnsealedTransactionV1::new(tx, signatures).into(),
        }
    }

    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        match self {
            Self::V1(ref mut tx) => {
                tx.set_dry_run(dry_run);
            },
        }
        self
    }

    pub fn finish(self) -> UnsealedTransaction {
        self.with_signatures(vec![])
    }

    pub(crate) fn inputs_mut(&mut self) -> &mut IndexSet<InputDeclaration> {
        match self {
            Self::V1(tx) => &mut tx.inputs,
        }
    }

    pub fn blobs(&self) -> &Blobs {
        match self {
            Self::V1(tx) => tx.blobs(),
        }
    }

    pub(crate) fn blobs_mut(&mut self) -> &mut Blobs {
        match self {
            Self::V1(tx) => tx.blobs_mut(),
        }
    }

    /// Append a blob and return its assigned index. Errors if blob count would overflow
    /// `BlobIndex` capacity.
    pub fn add_blob(&mut self, blob: Blob) -> Result<BlobIndex, BlobIndexOverflow> {
        self.blobs_mut().push(blob)
    }

    /// Validate blob index references and blob coverage. See
    /// `UnsignedTransactionV1::validate_blob_references`.
    pub fn validate_blob_references(&self) -> Result<(), crate::v1::BlobValidationError> {
        match self {
            Self::V1(tx) => tx.validate_blob_references(),
        }
    }
}

impl From<UnsignedTransactionV1> for UnsignedTransaction {
    fn from(tx: UnsignedTransactionV1) -> Self {
        Self::V1(tx)
    }
}

impl Signable<&RistrettoPublicKeyBytes> for UnsignedTransaction {
    type MessageOutput = [u8; 64];
    type Signature = TransactionSignature;

    fn to_signing_message(&self, sealed_signer: &RistrettoPublicKeyBytes) -> Self::MessageOutput {
        match &self {
            Self::V1(tx) => tx.to_signing_message(sealed_signer),
        }
    }
}

impl IntoSigned<&RistrettoPublicKeyBytes> for UnsignedTransaction {
    type SignedOutput = UnsealedTransaction;

    fn into_signed(self, sig: TransactionSignature) -> Self::SignedOutput {
        self.add_single_signature(sig)
    }
}
