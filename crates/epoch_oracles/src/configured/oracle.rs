//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeMap, VecDeque},
    future::poll_fn,
    iter,
    task::{Context, Poll},
};

use anyhow::Context as _;
use log::*;
use tari_epoch_manager::epoch_event_oracle::{EpochEvent, EpochEventOracle, ValidatorNodeChange};
use tari_ootle_common_types::{Epoch, SubstateAddress, displayable::Displayable};
use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

use super::config::Config;
use crate::{
    configured::{EpochTickerData, epoch_ticker::EpochTicker, real_time_ticker::RealTimeEpochTicker},
    store::{EpochOracleStore, StoreKey},
};

const LOG_TARGET: &str = "tari::ootle::epoch_oracles::configured";

/// A validator node registration to announce at a given epoch. Both a validator's initial registration and each of
/// its claim key rotations are emitted as a `ValidatorNodeChange::Add`, which is the only claim-key-bearing change
/// the base layer can express. A rotation reuses the registration's shard key so that changing the claim key does
/// not move the validator within its committee.
#[derive(Debug, Clone)]
struct QueuedActivation {
    public_key: RistrettoPublicKeyBytes,
    claim_key: RistrettoPublicKeyBytes,
    shard_key: SubstateAddress,
    kind: ActivationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationKind {
    Registration,
    ClaimKeyRotation,
}

pub struct ConfiguredEpochOracle<TStore, TTicker> {
    config: Config,
    pending_events: VecDeque<EpochEvent>,
    store: TStore,
    ticker: TTicker,
    queued_validators: BTreeMap<Epoch, Vec<QueuedActivation>>,
    is_initialized: bool,
    is_done: bool,
}
impl<TStore: EpochOracleStore + Send> ConfiguredEpochOracle<TStore, RealTimeEpochTicker> {
    pub fn create(config: Config, store: TStore) -> anyhow::Result<Self> {
        let epoch = store
            .get(StoreKey::ConfiguredCurrentEpoch.as_key_bytes())?
            .unwrap_or_else(Epoch::zero);
        let mut ticker = RealTimeEpochTicker::new(config.initial_epoch, config.base_time, epoch);
        if let Some(epoch_time) = config.epoch_time {
            ticker = ticker.with_epoch_time_secs(epoch_time.as_secs().try_into().context("Epoch time cannot be zero")?);
        } else {
            ticker = ticker.disable_ticks();
        }

        Ok(Self {
            config,
            store,
            ticker,
            pending_events: VecDeque::new(),
            queued_validators: BTreeMap::new(),
            is_initialized: false,
            is_done: false,
        })
    }
}

impl<TStore: EpochOracleStore + Send, TTicker: EpochTicker> ConfiguredEpochOracle<TStore, TTicker> {
    pub fn with_custom_ticker(config: Config, store: TStore, ticker: TTicker) -> Self {
        Self {
            config,
            store,
            ticker,
            pending_events: VecDeque::new(),
            queued_validators: BTreeMap::new(),
            is_initialized: false,
            is_done: false,
        }
    }

    /// Pins the shard key a validator activated with, and rejects a config that no longer derives it.
    ///
    /// The shard key is derived from config and written to the local validator_nodes table when a validator
    /// activates, so editing any field that feeds `Validator::calculate_shard_key` afterwards diverges the network
    /// silently: nodes holding the original row keep the original shard key while a node replaying the edited config
    /// from a fresh store derives a different one, and the two disagree on committee ordering with no epoch boundary
    /// at which to reconcile. Nothing downstream can attribute that, so refuse to start instead.
    fn verify_or_record_shard_key(
        &self,
        public_key: RistrettoPublicKeyBytes,
        shard_key: SubstateAddress,
    ) -> anyhow::Result<()> {
        let mut recorded = self
            .store
            .get::<Vec<(RistrettoPublicKeyBytes, SubstateAddress)>>(
                StoreKey::ConfiguredValidatorShardKeys.as_key_bytes(),
            )?
            .unwrap_or_default();

        match recorded.iter().find(|(pk, _)| *pk == public_key) {
            Some((_, activated_with)) if *activated_with != shard_key => {
                anyhow::bail!(
                    "Validator {public_key} activated with shard key {activated_with} but the current config derives \
                     {shard_key}. A validator's public_key, claim_key, shard_group and registration_epoch are fixed \
                     once it has activated; rotate its claim key with `claim_key_changes` instead of editing \
                     `claim_key`."
                );
            },
            Some(_) => Ok(()),
            None => {
                recorded.push((public_key, shard_key));
                self.store
                    .set(StoreKey::ConfiguredValidatorShardKeys.as_key_bytes(), &recorded)?;
                Ok(())
            },
        }
    }

    fn initialize(&mut self) -> anyhow::Result<()> {
        self.config.validate()?;

        let epoch = self
            .store
            .get(StoreKey::ConfiguredCurrentEpoch.as_key_bytes())?
            .unwrap_or_else(Epoch::zero);

        let mut skipped = 0usize;
        for vn in &self.config.validators {
            // Validators that activated before this node recorded shard keys are pinned to whatever the config
            // derives now. Their rows already exist, so this run's config is the best available record of what they
            // activated with, and it catches any subsequent edit.
            if vn.activation_epoch() <= epoch {
                self.verify_or_record_shard_key(vn.public_key, vn.calculate_shard_key())?;
            }
        }

        for vn in &self.config.validators {
            // A rotation carries the registration's shard key: the new claim key must not reach shard key
            // derivation, or the validator would move within its committee when its claim key changes.
            let shard_key = vn.calculate_shard_key();
            let activations = iter::once((vn.activation_epoch(), vn.claim_key, ActivationKind::Registration)).chain(
                vn.claim_key_changes.iter().map(|change| {
                    (
                        change.effective_epoch,
                        change.claim_key,
                        ActivationKind::ClaimKeyRotation,
                    )
                }),
            );

            for (activation_epoch, claim_key, kind) in activations {
                // Skip activations whose epoch has already been processed in a previous run.
                // Re-queuing them would emit a duplicate ActiveValidatorNodeSetChanged on the next tick,
                // causing duplicate rows in the validator_nodes table. Config changes are only supported
                // when starting from a fresh oracle store.
                if activation_epoch <= epoch {
                    skipped += 1;
                    continue;
                }

                let vns = self.queued_validators.entry(activation_epoch).or_default();
                vns.push(QueuedActivation {
                    public_key: vn.public_key,
                    claim_key,
                    shard_key,
                    kind,
                });
            }
        }

        info!(
            target: LOG_TARGET,
            "☘️ Starting Configured epoch oracle from {epoch}. Epoch time is {}. {} validator activation(s) queued, {skipped} already activated",
            self.config.epoch_time.as_ref().map(|d| d.display()).display(),
            self.queued_validators.values().map(|vns| vns.len()).sum::<usize>(),
        );

        Ok(())
    }

    fn prepare_next_epoch(&mut self, epoch_ticker_data: EpochTickerData) -> anyhow::Result<()> {
        let next_epoch = epoch_ticker_data.epoch;
        let epoch_hash = epoch_ticker_data.epoch_hash;
        if next_epoch.is_zero() {
            // If at epoch 0, just emit the epoch changed and done for now events
            self.pending_events.push_back(EpochEvent::EpochChanged {
                epoch: next_epoch,
                epoch_hash,
            });

            return Ok(());
        }

        let done_for_now = epoch_ticker_data.done_for_now;
        let current_epoch = next_epoch - Epoch(1);
        self.store
            .set(StoreKey::ConfiguredCurrentEpoch.as_key_bytes(), &next_epoch)?;

        debug!(
            target: LOG_TARGET,
            "☘️ Preparing next epoch {next_epoch}",
        );
        let next_next_epoch = next_epoch + Epoch(1);
        if let Some(vns) = self.queued_validators.get(&next_next_epoch) {
            debug!(
                target: LOG_TARGET,
                "☘️ {} VNS registered for epoch {next_next_epoch}",
                vns.len()
            );
            // Emit these one epoch before a VN becomes active. A claim key rotation is not a new registration, so
            // it is not announced.
            self.pending_events.extend(
                vns.iter()
                    .filter(|vn| vn.kind == ActivationKind::Registration)
                    .map(|vn| EpochEvent::NewValidatorRegistered {
                        epoch: next_epoch,
                        claim_public_key: vn.claim_key,
                        validator_node_public_key: vn.public_key,
                    }),
            )
        }

        // Activate all validators queued at or before next_epoch (handles skipped epochs).
        // split_off returns everything > next_epoch, leaving everything <= next_epoch in place.
        let remaining = self.queued_validators.split_off(&(next_epoch + Epoch(1)));
        let due = std::mem::replace(&mut self.queued_validators, remaining);

        for (epoch, vns) in due {
            debug!(
                target: LOG_TARGET,
                "☘️ {} VNS activated for epoch {epoch} (current: {next_epoch})",
                vns.len()
            );
            // Pin each shard key as it is committed, so that a later edit to the config that derived it is caught on
            // the next startup even if this node never restarted between the two.
            for vn in vns.iter().filter(|vn| vn.kind == ActivationKind::Registration) {
                self.verify_or_record_shard_key(vn.public_key, vn.shard_key)?;
            }
            self.pending_events
                .push_back(EpochEvent::ActiveValidatorNodeSetChanged {
                    epoch: current_epoch,
                    node_changes: vns
                        .into_iter()
                        .map(|vn| ValidatorNodeChange::Add {
                            claim_public_key: vn.claim_key,
                            validator_node_public_key: vn.public_key,
                            activation_epoch: next_epoch,
                            minimum_value_promise: 0,
                            shard_key: vn.shard_key,
                        })
                        .collect(),
                });
        }

        self.pending_events.push_back(EpochEvent::EpochChanged {
            epoch: next_epoch,
            epoch_hash,
        });

        if done_for_now {
            self.pending_events.push_back(EpochEvent::DoneForNow {
                epoch: next_epoch,
                epoch_hash,
            });
        }

        Ok(())
    }

    fn poll(&mut self, cx: &mut Context) -> Poll<Option<EpochEvent>> {
        if self.is_done {
            return Poll::Ready(None);
        }

        if !self.is_initialized {
            if let Err(err) = self.initialize() {
                self.is_done = true;
                return Poll::Ready(Some(EpochEvent::error(err)));
            }
            self.is_initialized = true;
        }

        loop {
            if let Some(event) = self.pending_events.pop_front() {
                return Poll::Ready(Some(event));
            }

            match self.ticker.poll_tick(cx) {
                Poll::Ready(Some(data)) => {
                    info!(target: LOG_TARGET, "⏰ Ticker ticked for epoch {}, done_for_now = {}", data.epoch, data.done_for_now);
                    if let Err(err) = self.prepare_next_epoch(data) {
                        self.is_done = true;
                        return Poll::Ready(Some(EpochEvent::error(err)));
                    }
                },
                Poll::Ready(None) => {
                    debug!(target: LOG_TARGET, "Ticker returned None");
                    self.is_done = true;
                    return Poll::Ready(None);
                },
                Poll::Pending => {
                    // Still waiting for the next tick
                    return Poll::Pending;
                },
            }
        }
    }
}

impl<TStore: EpochOracleStore + Send, TTicker: EpochTicker + Send> EpochEventOracle
    for ConfiguredEpochOracle<TStore, TTicker>
{
    async fn next_epoch_event(&mut self) -> Option<EpochEvent> {
        poll_fn(|cx| self.poll(cx)).await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        sync::Mutex,
        task::{Context, Poll, Waker},
        time::Duration,
    };

    use serde::{Serialize, de::DeserializeOwned};
    use tari_epoch_manager::epoch_event_oracle::{EpochEvent, ValidatorNodeChange};
    use tari_ootle_common_types::{Epoch, ShardGroup, SubstateAddress};
    use tari_sidechain::FixedHash;
    use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

    use super::ConfiguredEpochOracle;
    use crate::{
        configured::{ClaimKeyChange, Config, EpochTicker, EpochTickerData, Validator},
        store::{EpochOracleStore, StoreKey},
    };

    #[derive(Default)]
    struct TestStore {
        data: Mutex<HashMap<Vec<u8>, Vec<u8>>>,
    }

    impl EpochOracleStore for TestStore {
        fn get<T: DeserializeOwned>(&self, key: &[u8]) -> anyhow::Result<Option<T>> {
            let data = self.data.lock().unwrap();
            data.get(key).map(|v| Ok(serde_json::from_slice(v)?)).transpose()
        }

        fn set<T: Serialize>(&self, key: &[u8], value: &T) -> anyhow::Result<()> {
            let mut data = self.data.lock().unwrap();
            data.insert(key.to_vec(), serde_json::to_vec(value)?);
            Ok(())
        }
    }

    struct ScriptedTicker {
        data: VecDeque<EpochTickerData>,
    }

    impl ScriptedTicker {
        fn new(data: Vec<EpochTickerData>) -> Self {
            Self { data: data.into() }
        }
    }

    impl EpochTicker for ScriptedTicker {
        fn poll_tick(&mut self, _cx: &mut Context) -> Poll<Option<EpochTickerData>> {
            match self.data.pop_front() {
                Some(d) => Poll::Ready(Some(d)),
                None => Poll::Pending,
            }
        }
    }

    fn mk_key(seed: u8) -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(&[seed; 32]).unwrap()
    }

    fn mk_validator(public_seed: u8, registration_epoch: Epoch) -> Validator {
        Validator {
            public_key: mk_key(public_seed),
            claim_key: mk_key(public_seed.wrapping_add(1)),
            shard_group: ShardGroup::new(1, 256),
            registration_epoch,
            claim_key_changes: vec![],
        }
    }

    /// The `Add` changes emitted by each `ActiveValidatorNodeSetChanged`, flattened to
    /// (announcing epoch, claim key, activation epoch, shard key).
    fn added_validators(events: &[EpochEvent]) -> Vec<(Epoch, RistrettoPublicKeyBytes, Epoch, SubstateAddress)> {
        events
            .iter()
            .filter_map(|e| match e {
                EpochEvent::ActiveValidatorNodeSetChanged { epoch, node_changes } => Some((*epoch, node_changes)),
                _ => None,
            })
            .flat_map(|(epoch, node_changes)| {
                node_changes.iter().map(move |change| match change {
                    ValidatorNodeChange::Add {
                        claim_public_key,
                        activation_epoch,
                        shard_key,
                        ..
                    } => (epoch, *claim_public_key, *activation_epoch, *shard_key),
                    ValidatorNodeChange::Remove { .. } => panic!("unexpected Remove change"),
                })
            })
            .collect()
    }

    fn ticker_through(last_epoch: u64) -> ScriptedTicker {
        ScriptedTicker::new(
            (0..=last_epoch)
                .map(|e| EpochTickerData {
                    epoch: Epoch(e),
                    epoch_hash: FixedHash::zero(),
                    done_for_now: e == last_epoch,
                })
                .collect(),
        )
    }

    fn mk_config(validators: Vec<Validator>) -> Config {
        Config {
            epoch_time: Some(Duration::from_secs(1)),
            initial_epoch: Epoch(0),
            base_time: time::OffsetDateTime::now_utc(),
            validators,
        }
    }

    fn drive_events<T: EpochOracleStore + Send, TT: EpochTicker>(
        oracle: &mut ConfiguredEpochOracle<T, TT>,
    ) -> Vec<EpochEvent> {
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut events = vec![];
        while let Poll::Ready(Some(event)) = oracle.poll(&mut cx) {
            events.push(event);
        }
        events
    }

    #[test]
    fn restart_does_not_reactivate_already_activated_validators() {
        let config = mk_config(vec![mk_validator(1, Epoch(10))]);
        let store = TestStore::default();
        // Simulate prior run having processed the validator's activation_epoch (11) already.
        store
            .set(StoreKey::ConfiguredCurrentEpoch.as_key_bytes(), &Epoch(11))
            .unwrap();

        // Ticker re-emits the last-processed epoch on restart.
        let ticker = ScriptedTicker::new(vec![EpochTickerData {
            epoch: Epoch(11),
            epoch_hash: FixedHash::zero(),
            done_for_now: true,
        }]);
        let mut oracle = ConfiguredEpochOracle::with_custom_ticker(config, store, ticker);

        let events = drive_events(&mut oracle);

        assert!(
            !events
                .iter()
                .any(|e| matches!(e, EpochEvent::ActiveValidatorNodeSetChanged { .. })),
            "restart must not re-emit ActiveValidatorNodeSetChanged; got {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, EpochEvent::NewValidatorRegistered { .. })),
            "restart must not re-emit NewValidatorRegistered; got {events:?}"
        );
    }

    #[test]
    fn fresh_start_activates_validators_at_activation_epoch() {
        let config = mk_config(vec![mk_validator(1, Epoch(10))]);
        let store = TestStore::default();
        let ticker = ScriptedTicker::new(
            (0..=11)
                .map(|e| EpochTickerData {
                    epoch: Epoch(e),
                    epoch_hash: FixedHash::zero(),
                    done_for_now: e == 11,
                })
                .collect(),
        );
        let mut oracle = ConfiguredEpochOracle::with_custom_ticker(config, store, ticker);

        let events = drive_events(&mut oracle);

        let activations: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                EpochEvent::ActiveValidatorNodeSetChanged { epoch, node_changes } => Some((*epoch, node_changes.len())),
                _ => None,
            })
            .collect();
        assert_eq!(
            activations,
            vec![(Epoch(10), 1)],
            "validator should activate exactly once, announced at epoch 10"
        );

        let announces = events
            .iter()
            .filter(|e| matches!(e, EpochEvent::NewValidatorRegistered { .. }))
            .count();
        assert_eq!(announces, 1, "should announce exactly once");
    }

    #[test]
    fn config_added_after_activation_epoch_is_skipped_on_restart() {
        // User adds a validator with a past registration_epoch and restarts without clearing
        // the oracle store. We skip rather than retroactively activate, since config changes
        // are only supported when starting from a fresh store.
        let config = mk_config(vec![mk_validator(1, Epoch(5))]);
        let store = TestStore::default();
        store
            .set(StoreKey::ConfiguredCurrentEpoch.as_key_bytes(), &Epoch(20))
            .unwrap();

        let ticker = ScriptedTicker::new(vec![EpochTickerData {
            epoch: Epoch(21),
            epoch_hash: FixedHash::zero(),
            done_for_now: true,
        }]);
        let mut oracle = ConfiguredEpochOracle::with_custom_ticker(config, store, ticker);

        let events = drive_events(&mut oracle);

        assert!(
            !events
                .iter()
                .any(|e| matches!(e, EpochEvent::ActiveValidatorNodeSetChanged { .. })),
            "stale-config validator must not be retroactively activated; got {events:?}"
        );
    }

    fn mk_validator_with_rotation(registration_epoch: Epoch, effective_epoch: Epoch) -> Validator {
        Validator {
            claim_key_changes: vec![ClaimKeyChange {
                effective_epoch,
                claim_key: mk_key(99),
            }],
            ..mk_validator(1, registration_epoch)
        }
    }

    #[test]
    fn claim_key_rotation_re_adds_validator_with_new_key_and_same_shard_key() {
        let validator = mk_validator_with_rotation(Epoch(10), Epoch(20));
        let expected_shard_key = validator.calculate_shard_key();
        let original_claim_key = validator.claim_key;
        let config = mk_config(vec![validator]);
        let mut oracle = ConfiguredEpochOracle::with_custom_ticker(config, TestStore::default(), ticker_through(20));

        let events = drive_events(&mut oracle);

        assert_eq!(
            added_validators(&events),
            vec![
                (Epoch(10), original_claim_key, Epoch(11), expected_shard_key),
                (Epoch(19), mk_key(99), Epoch(20), expected_shard_key),
            ],
            "rotation should re-add the validator at its effective epoch with the new claim key and the \
             registration's shard key"
        );
    }

    #[test]
    fn claim_key_rotation_is_not_announced_as_a_new_registration() {
        let config = mk_config(vec![mk_validator_with_rotation(Epoch(10), Epoch(20))]);
        let mut oracle = ConfiguredEpochOracle::with_custom_ticker(config, TestStore::default(), ticker_through(20));

        let events = drive_events(&mut oracle);

        let announced = events
            .iter()
            .filter(|e| matches!(e, EpochEvent::NewValidatorRegistered { .. }))
            .count();
        assert_eq!(
            announced, 1,
            "only the initial registration should be announced; got {events:?}"
        );
    }

    #[test]
    fn restart_before_effective_epoch_still_emits_the_rotation() {
        let config = mk_config(vec![mk_validator_with_rotation(Epoch(10), Epoch(20))]);
        let store = TestStore::default();
        // A prior run activated the validator but has not yet reached the rotation.
        store
            .set(StoreKey::ConfiguredCurrentEpoch.as_key_bytes(), &Epoch(15))
            .unwrap();
        let mut oracle = ConfiguredEpochOracle::with_custom_ticker(config, store, ticker_through(20));

        let events = drive_events(&mut oracle);

        assert_eq!(
            added_validators(&events)
                .into_iter()
                .map(|(_, claim_key, activation_epoch, _)| (claim_key, activation_epoch))
                .collect::<Vec<_>>(),
            vec![(mk_key(99), Epoch(20))],
            "the already-activated registration must not be re-emitted, but the pending rotation must be"
        );
    }

    #[test]
    fn restart_after_effective_epoch_does_not_re_emit_the_rotation() {
        let config = mk_config(vec![mk_validator_with_rotation(Epoch(10), Epoch(20))]);
        let store = TestStore::default();
        store
            .set(StoreKey::ConfiguredCurrentEpoch.as_key_bytes(), &Epoch(25))
            .unwrap();
        let ticker = ScriptedTicker::new(vec![EpochTickerData {
            epoch: Epoch(25),
            epoch_hash: FixedHash::zero(),
            done_for_now: true,
        }]);
        let mut oracle = ConfiguredEpochOracle::with_custom_ticker(config, store, ticker);

        let events = drive_events(&mut oracle);

        assert!(
            added_validators(&events).is_empty(),
            "an already-applied rotation must not be re-emitted; got {events:?}"
        );
    }

    #[test]
    fn fresh_store_replay_past_the_rotation_converges_on_the_new_key() {
        let config = mk_config(vec![mk_validator_with_rotation(Epoch(10), Epoch(20))]);
        // A node syncing from scratch replays every epoch, so it must arrive at the same final claim key as a node
        // that was live across the boundary.
        let mut oracle = ConfiguredEpochOracle::with_custom_ticker(config, TestStore::default(), ticker_through(30));

        let events = drive_events(&mut oracle);

        let (_, final_claim_key, final_activation_epoch, _) =
            *added_validators(&events).last().expect("at least one Add");
        assert_eq!(final_claim_key, mk_key(99));
        assert_eq!(final_activation_epoch, Epoch(20));
    }

    #[test]
    fn editing_the_claim_key_of_an_activated_validator_is_rejected() {
        let store = TestStore::default();
        // A prior run activated the validator and pinned the shard key it derived.
        store
            .set(StoreKey::ConfiguredCurrentEpoch.as_key_bytes(), &Epoch(15))
            .unwrap();
        let original = mk_validator(1, Epoch(10));
        store
            .set(StoreKey::ConfiguredValidatorShardKeys.as_key_bytes(), &vec![(
                original.public_key,
                original.calculate_shard_key(),
            )])
            .unwrap();

        // The operator rotates by editing `claim_key` in place rather than adding a `claim_key_changes` entry. This
        // is silently a no-op here but would derive a different shard key on a node replaying from a fresh store.
        let edited = Validator {
            claim_key: mk_key(99),
            ..original
        };
        let mut oracle = ConfiguredEpochOracle::with_custom_ticker(mk_config(vec![edited]), store, ticker_through(20));

        let events = drive_events(&mut oracle);

        match events.as_slice() {
            [EpochEvent::Error(err)] => {
                let err = err.to_string();
                assert!(err.contains("activated with shard key"), "{err}");
            },
            _ => panic!("editing an activated validator's claim key must be rejected; got {events:?}"),
        }
    }

    #[test]
    fn a_scheduled_rotation_does_not_trip_the_shard_key_check() {
        let store = TestStore::default();
        store
            .set(StoreKey::ConfiguredCurrentEpoch.as_key_bytes(), &Epoch(15))
            .unwrap();
        let validator = mk_validator_with_rotation(Epoch(10), Epoch(20));
        store
            .set(StoreKey::ConfiguredValidatorShardKeys.as_key_bytes(), &vec![(
                validator.public_key,
                // Pinned from the config before the rotation was added.
                mk_validator(1, Epoch(10)).calculate_shard_key(),
            )])
            .unwrap();
        let mut oracle =
            ConfiguredEpochOracle::with_custom_ticker(mk_config(vec![validator]), store, ticker_through(20));

        let events = drive_events(&mut oracle);

        assert!(
            !events.iter().any(|e| matches!(e, EpochEvent::Error(_))),
            "adding a claim key rotation must not change the shard key; got {events:?}"
        );
        assert_eq!(
            added_validators(&events)
                .into_iter()
                .map(|(_, claim_key, _, _)| claim_key)
                .collect::<Vec<_>>(),
            vec![mk_key(99)]
        );
    }

    #[test]
    fn invalid_config_stops_the_oracle_with_an_error() {
        let mut validator = mk_validator(1, Epoch(10));
        // Rotating before the validator activates has no row to supersede.
        validator.claim_key_changes = vec![ClaimKeyChange {
            effective_epoch: Epoch(5),
            claim_key: mk_key(99),
        }];
        let config = mk_config(vec![validator]);
        let mut oracle = ConfiguredEpochOracle::with_custom_ticker(config, TestStore::default(), ticker_through(20));

        let events = drive_events(&mut oracle);

        assert!(
            matches!(events.as_slice(), [EpochEvent::Error(_)]),
            "an invalid config must surface as an error, not a partial event stream; got {events:?}"
        );
    }
}
