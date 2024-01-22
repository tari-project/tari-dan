//   Copyright 2023. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

/// A tag applied to various engine types. We use an unassigned CBOR tag range (128 to 255 inclusive). <https://www.iana.org/assignments/cbor-tags/cbor-tags.xhtml>
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum BinaryTag {
    ComponentAddress = 128,
    Metadata = 129,
    NonFungibleAddress = 130,
    ResourceAddress = 131,
    VaultId = 132,
    BucketId = 133,
    TransactionReceipt = 134,
    FeeClaim = 135,
    ProofId = 136,
}

impl BinaryTag {
    pub fn from_u64(value: u64) -> Option<Self> {
        match value {
            128 => Some(Self::ComponentAddress),
            129 => Some(Self::Metadata),
            130 => Some(Self::NonFungibleAddress),
            131 => Some(Self::ResourceAddress),
            132 => Some(Self::VaultId),
            133 => Some(Self::BucketId),
            134 => Some(Self::TransactionReceipt),
            135 => Some(Self::FeeClaim),
            136 => Some(Self::ProofId),
            _ => None,
        }
    }

    pub const fn as_u64(&self) -> u64 {
        *self as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_and_as_u64_parity() {
        let cases = &[
            BinaryTag::ComponentAddress,
            BinaryTag::Metadata,
            BinaryTag::NonFungibleAddress,
            BinaryTag::ResourceAddress,
            BinaryTag::VaultId,
            BinaryTag::BucketId,
            BinaryTag::TransactionReceipt,
            BinaryTag::FeeClaim,
            BinaryTag::ProofId,
        ];

        for case in cases {
            assert_eq!(BinaryTag::from_u64(case.as_u64()).unwrap().as_u64(), case.as_u64());
        }
    }
}
