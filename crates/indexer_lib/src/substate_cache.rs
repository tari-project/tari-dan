// Copyright 2023. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::future::Future;

use tari_engine_types::substate::SubstateId;
use tari_validator_node_rpc::client::SubstateResult;

#[derive(thiserror::Error, Debug)]
#[error("Failed substate cache operation {0}")]
pub struct SubstateCacheError(pub String);

/// A point in a shard's state transition stream up to which the cache holds every transition.
///
/// The cache is served on a completeness argument rather than a timer: an entry answers for the
/// substate's latest version because every commit that would supersede or destroy it reaches the
/// cache through that shard's stream. A watermark is what makes the argument checkable — it is
/// captured before a committee fetch and handed back to [`SubstateCache::write`], so a transition
/// that arrived while the fetch was in flight can veto the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FetchWatermark(u64);

impl FetchWatermark {
    pub const fn new(state_version: u64) -> Self {
        Self(state_version)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Whether the cache holds `DoesNotExist` for this substate.
///
/// A cached negative saves the most expensive lookup there is: `DoesNotExist` is the one answer that
/// cannot short-circuit on the first good response, so it costs a walk of `f + 1` committee members
/// every time. It is correct only while the transition stream can retract it, and each retraction
/// costs a journal row for a creation that would otherwise write none, so it is worth holding only
/// where the saving is repeated.
///
/// Transaction receipts are excluded. They are created once and never updated - the bulk of the
/// stream by count - so they carry nearly all of that journal cost, and a receipt the indexer has
/// synced is answered from its own tables without a committee walk at all. What is left is the
/// caller polling for a receipt that does not exist yet, which is the one case a cached negative
/// delays rather than serves.
pub fn caches_nonexistence(id: &SubstateId) -> bool {
    !id.is_transaction_receipt()
}

#[derive(Debug, Clone)]
pub struct SubstateCacheEntry {
    /// The substate's head version, or `None` when the substate does not exist.
    pub version: Option<u32>,
    pub substate_result: SubstateResult,
    pub cached_at: u64,
    /// True if the value was committee-verified when it was fetched. Never true for a substate that
    /// does not exist: absence has nothing to prove against the state tree.
    pub verified: bool,
}

impl SubstateCacheEntry {
    /// What this head says about a lookup at `version`, or `None` when it says nothing.
    ///
    /// The indexer serves the network's current state, not its history, so a version is only ever
    /// asked about to learn whether it is still current. The head answers that on its own: the
    /// version named is the head, or it is below it and therefore down - versions are contiguous
    /// and upping a substate downs its predecessor - or it is above it, which the cache knows
    /// nothing about. Nonexistence names no version and answers nothing about one: a destroyed
    /// substate whose history has been pruned reports the same thing as one never created.
    pub fn answer_at(self, version: Option<u32>) -> Option<Self> {
        let Some(version) = version else {
            return Some(self);
        };
        let head = self.version?;
        if version == head {
            return Some(self);
        }
        (version < head).then(|| Self {
            version: Some(version),
            substate_result: SubstateResult::Down { version },
            ..self
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SubstateCacheEntryRef<'a> {
    /// The substate's head version, or `None` when the substate does not exist.
    pub version: Option<u32>,
    pub substate_result: &'a SubstateResult,
    pub cached_at: u64,
    pub verified: bool,
}

pub trait SubstateCache: Send + Sync {
    /// The watermark for the shard owning `id`, or `None` when that shard's completeness cannot be
    /// established - it has never been synced, or the last sync of it is too far behind to serve
    /// from. Nothing may be cached for a substate whose shard has no watermark.
    fn watermark(
        &self,
        id: &SubstateId,
    ) -> impl Future<Output = Result<Option<FetchWatermark>, SubstateCacheError>> + Send;

    /// The cached head of `id`, or `None` when nothing is cached or the shard has no watermark. What
    /// the head says about a particular version is [`SubstateCacheEntry::answer_at`]'s to decide.
    fn read(
        &self,
        id: &SubstateId,
    ) -> impl Future<Output = Result<Option<SubstateCacheEntry>, SubstateCacheError>> + Send;

    /// Records `entry` as the substate's head version, provided no transition for `id` has arrived
    /// since `watermark`. A write vetoed that way is not an error: the caller still has its freshly
    /// fetched value, and the next read fetches again.
    ///
    /// The cache holds one entry per substate, its head. Only a result that establishes the head may
    /// be written: an `Up` version is always the head, since upping a substate downs its predecessor,
    /// and a lookup that named no version answers with the head by definition. A named version that
    /// came back `Down` establishes only that the head is higher, not what it is.
    ///
    /// An entry with no version records that the substate does not exist, which an unversioned
    /// lookup establishes as much as any version does. Only substates [`caches_nonexistence`] admits
    /// may be written that way: the stream retracts such an entry through a journal row that a
    /// creation writes for no other reason.
    fn write(
        &self,
        id: &SubstateId,
        entry: SubstateCacheEntryRef<'_>,
        watermark: FetchWatermark,
    ) -> impl Future<Output = Result<(), SubstateCacheError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(version: Option<u32>) -> SubstateCacheEntry {
        SubstateCacheEntry {
            version,
            substate_result: version.map_or(SubstateResult::DoesNotExist, |version| SubstateResult::Down { version }),
            cached_at: 1_000,
            verified: true,
        }
    }

    #[test]
    fn the_head_answers_an_unversioned_read_and_itself() {
        assert_eq!(head(Some(6)).answer_at(None).unwrap().version, Some(6));
        assert_eq!(head(Some(6)).answer_at(Some(6)).unwrap().version, Some(6));
    }

    /// The head only has to have been real at some point: the real head is at or above it, so every
    /// version below is down for good. The answer carries the head's age so that it ages out with it.
    #[test]
    fn a_version_below_the_head_is_down() {
        let answer = head(Some(6)).answer_at(Some(3)).unwrap();
        assert!(matches!(answer.substate_result, SubstateResult::Down { version: 3 }));
        assert_eq!(answer.version, Some(3));
        assert_eq!(answer.cached_at, 1_000);
        assert!(answer.verified);
    }

    /// Above the head the cache knows nothing: this indexer is behind, or the substate never got there.
    #[test]
    fn a_version_above_the_head_is_a_miss() {
        assert!(head(Some(6)).answer_at(Some(7)).is_none());
    }

    #[test]
    fn nonexistence_answers_an_unversioned_read_only() {
        assert!(head(None).answer_at(None).is_some());
        assert!(head(None).answer_at(Some(0)).is_none());
        assert!(head(None).answer_at(Some(3)).is_none());
    }
}
