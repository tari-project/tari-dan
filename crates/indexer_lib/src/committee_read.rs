//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::future::Future;

use futures::{StreamExt, stream::FuturesUnordered};
use tari_validator_node_rpc::client::SubstateResult;

use crate::{cached_substate_manager::SubstateLookupResult, error::IndexerError};

const LOG_TARGET: &str = "tari::indexer::scanner";

/// How many committee members a single-substate read keeps in flight at once.
///
/// A member that is down costs a full connect timeout before it answers with an error, so a read
/// that asks one member at a time stalls for that long whenever the first pick is dead. Asking a few
/// at once bounds the stall to the slowest of the in-flight members that are actually up, at the
/// cost of that many concurrent requests per read.
pub const READ_RACE_WIDTH: usize = 3;

/// One committee member's answer to a substate read: the result and whether it came with a proof
/// that verified against the committee.
pub type MemberResponse = Result<(SubstateResult, bool), IndexerError>;

/// Folds committee members' responses to a single-substate read into an answer.
///
/// Responses arrive in whatever order the members answer, so the tally cannot assume anything about
/// which member it hears from first. What settles a read is decided per response: a proven `Up` or
/// `Down` (or any `Up`/`Down` while proofs are not required) answers on the spot, `DoesNotExist`
/// answers only once more than `f` members agree, and an unproven `Up`/`Down` is held as a fallback
/// in case no member can prove one.
#[derive(Debug)]
pub struct CommitteeReadTally {
    /// Byzantine tolerance of the committee: `DoesNotExist` needs `f + 1` agreeing members before it
    /// is believed, since any `f` of them may be lying or behind.
    f: usize,
    verify_substate_proofs: bool,
    num_nexist: usize,
    last_error: Option<IndexerError>,
    /// Highest-version `Up`/`Down` response that came back without a proof. Only served if no
    /// member can prove one.
    unproven_result: Option<SubstateResult>,
}

impl CommitteeReadTally {
    pub fn new(committee_size: usize, verify_substate_proofs: bool) -> Self {
        Self {
            f: committee_size.saturating_sub(1) / 3,
            verify_substate_proofs,
            num_nexist: 0,
            last_error: None,
            unproven_result: None,
        }
    }

    /// Folds in one member's response, returning the answer if this response settles the read.
    pub fn observe(&mut self, response: MemberResponse) -> Option<SubstateLookupResult> {
        match response {
            Ok((substate_result, verified)) => match substate_result {
                SubstateResult::Up { .. } | SubstateResult::Down { .. } => {
                    if verified || !self.verify_substate_proofs {
                        return Some(SubstateLookupResult {
                            result: substate_result,
                            verified,
                        });
                    }
                    // The member could not prove its response (e.g. nothing committed since the
                    // epoch started). Keep the highest version as a fallback (a member that is still
                    // syncing may respond with a stale copy) and wait on the rest of the committee
                    // for a proven copy.
                    if self
                        .unproven_result
                        .as_ref()
                        .is_none_or(|r| r.version() < substate_result.version())
                    {
                        self.unproven_result = Some(substate_result);
                    }
                    None
                },
                SubstateResult::DoesNotExist => {
                    self.num_nexist += 1;
                    (self.num_nexist > self.f).then_some(SubstateLookupResult {
                        result: SubstateResult::DoesNotExist,
                        verified: false,
                    })
                },
            },
            Err(e) => {
                // A single member's error is ignored while the rest of the committee may still answer.
                self.last_error = Some(e);
                None
            },
        }
    }

    /// The answer once every member has responded without settling the read.
    pub fn conclude(self, describe: impl std::fmt::Display) -> Result<SubstateLookupResult, IndexerError> {
        if let Some(result) = self.unproven_result {
            log::warn!(
                target: LOG_TARGET,
                "No committee member could supply a proof for {describe}. Returning the substate unverified.",
            );
            return Ok(SubstateLookupResult {
                result,
                verified: false,
            });
        }

        log::warn!(
            target: LOG_TARGET,
            "Could not get substate {describe} from any of the validator nodes",
        );

        if let Some(e) = self.last_error {
            return Err(e);
        }
        Ok(SubstateLookupResult {
            result: SubstateResult::DoesNotExist,
            verified: false,
        })
    }
}

/// Drives one request per committee member, at most `width` at a time, until a response settles
/// the read.
///
/// `requests` is drawn from lazily: a member is asked as soon as a slot frees up, so an unresponsive
/// member holds one slot for as long as its request takes and delays nothing else. Requests still
/// in flight when the read settles are dropped.
pub async fn race_committee<I, D>(
    requests: I,
    width: usize,
    mut tally: CommitteeReadTally,
    describe: D,
) -> Result<SubstateLookupResult, IndexerError>
where
    I: IntoIterator,
    I::Item: Future<Output = MemberResponse>,
    D: std::fmt::Display,
{
    let mut requests = requests.into_iter();
    let mut in_flight = FuturesUnordered::new();
    in_flight.extend(requests.by_ref().take(width.max(1)));

    while let Some(response) = in_flight.next().await {
        if let Some(answer) = tally.observe(response) {
            return Ok(answer);
        }
        if let Some(request) = requests.next() {
            in_flight.push(request);
        }
    }

    tally.conclude(describe)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use futures::future::{BoxFuture, FutureExt};

    use super::*;

    fn down(version: u32) -> SubstateResult {
        SubstateResult::Down { version }
    }

    fn error() -> IndexerError {
        IndexerError::ValidatorNodeClientError("unreachable".into())
    }

    /// A committee whose member `n` answers with `responses[n]`, or never answers when there is no
    /// entry for it.
    struct Committee {
        responses: HashMap<usize, MemberResponse>,
        started: Arc<AtomicUsize>,
    }

    impl Committee {
        fn new(responses: Vec<(usize, MemberResponse)>) -> Self {
            Self {
                responses: responses.into_iter().collect(),
                started: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn fetch(&mut self) -> impl Fn(usize) -> BoxFuture<'static, MemberResponse> + use<> {
            let started = self.started.clone();
            // Each member answers at most once, so its response is handed over rather than cloned.
            let responses = Arc::new(std::sync::Mutex::new(std::mem::take(&mut self.responses)));
            move |member| {
                started.fetch_add(1, Ordering::SeqCst);
                match responses.lock().unwrap().remove(&member) {
                    Some(response) => futures::future::ready(response).boxed(),
                    None => futures::future::pending().boxed(),
                }
            }
        }
    }

    async fn race(
        size: usize,
        width: usize,
        verify: bool,
        responses: Vec<(usize, MemberResponse)>,
    ) -> Result<SubstateLookupResult, IndexerError> {
        let mut committee = Committee::new(responses);
        let fetch = committee.fetch();
        race_committee(
            (0..size).map(fetch),
            width,
            CommitteeReadTally::new(size, verify),
            "test",
        )
        .await
    }

    #[tokio::test]
    async fn a_member_that_never_answers_does_not_delay_the_rest() {
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            race(3, READ_RACE_WIDTH, true, vec![(1, Ok((down(4), true)))]),
        )
        .await
        .expect("read stalled behind an unresponsive member")
        .unwrap();
        assert_eq!(result.result.version(), Some(4));
        assert!(result.verified);
    }

    #[tokio::test]
    async fn no_more_than_the_window_is_in_flight() {
        let mut committee = Committee::new(vec![]);
        let started = committee.started.clone();
        let fetch = committee.fetch();
        let read = race_committee((0..5).map(fetch), 2, CommitteeReadTally::new(5, true), "test");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), read).await.is_err(),
            "nothing answered, so the read cannot have settled"
        );
        assert_eq!(started.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_member_frees_its_slot_for_the_next() {
        let result = race(3, 1, true, vec![
            (0, Err(error())),
            (1, Err(error())),
            (2, Ok((down(2), true))),
        ])
        .await
        .unwrap();
        assert_eq!(result.result.version(), Some(2));
    }

    #[tokio::test]
    async fn an_unproven_answer_is_held_until_a_proof_arrives() {
        // Member 0 answers first and cannot prove; member 1 can.
        let result = race(2, 1, true, vec![(0, Ok((down(7), false))), (1, Ok((down(7), true)))])
            .await
            .unwrap();
        assert!(result.verified);
    }

    #[tokio::test]
    async fn the_highest_unproven_version_is_served_when_nobody_can_prove() {
        let result = race(3, 1, true, vec![
            (0, Ok((down(2), false))),
            (1, Ok((down(5), false))),
            (2, Ok((down(3), false))),
        ])
        .await
        .unwrap();
        assert_eq!(result.result.version(), Some(5));
        assert!(!result.verified);
    }

    #[tokio::test]
    async fn an_unproven_answer_settles_the_read_when_proofs_are_not_required() {
        let result = race(2, 1, false, vec![(0, Ok((down(1), false)))]).await.unwrap();
        assert_eq!(result.result.version(), Some(1));
        assert!(!result.verified);
    }

    #[tokio::test]
    async fn nonexistence_needs_more_than_f_agreeing_members() {
        // Four members: f = 1, so two must agree.
        let result = race(4, 1, true, vec![
            (0, Ok((SubstateResult::DoesNotExist, false))),
            (1, Ok((SubstateResult::DoesNotExist, false))),
        ])
        .await
        .unwrap();
        assert!(matches!(result.result, SubstateResult::DoesNotExist));
    }

    #[tokio::test]
    async fn a_single_nonexistence_is_outvoted_by_a_proven_version() {
        let result = race(4, 1, true, vec![
            (0, Ok((SubstateResult::DoesNotExist, false))),
            (1, Ok((down(1), true))),
        ])
        .await
        .unwrap();
        assert_eq!(result.result.version(), Some(1));
    }

    #[tokio::test]
    async fn f_agreeing_members_and_errors_from_the_rest_is_the_last_error() {
        let result = race(4, 1, true, vec![
            (0, Ok((SubstateResult::DoesNotExist, false))),
            (1, Err(error())),
            (2, Err(error())),
            (3, Err(error())),
        ])
        .await;
        assert!(matches!(result, Err(IndexerError::ValidatorNodeClientError(_))));
    }

    /// Agreement takes precedence over errors from members that could not be reached.
    #[tokio::test]
    async fn agreed_nonexistence_outranks_errors() {
        let result = race(4, 1, true, vec![
            (0, Err(error())),
            (1, Ok((SubstateResult::DoesNotExist, false))),
            (2, Err(error())),
            (3, Ok((SubstateResult::DoesNotExist, false))),
        ])
        .await
        .unwrap();
        assert!(matches!(result.result, SubstateResult::DoesNotExist));
    }
}
