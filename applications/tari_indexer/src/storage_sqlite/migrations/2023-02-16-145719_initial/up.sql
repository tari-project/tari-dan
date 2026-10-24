create table substates
(
    id               integer   not NULL primary key AUTOINCREMENT,
    address          text      not NULL,
    version          bigint    not NULL,
    -- The CBOR-encoded `SubstateValue`.
    data             blob      not NULL,
    template_address text      NULL,
    module_name      text      NULL,
    updated_at       timestamp not null default current_timestamp,
    created_at       timestamp not null default current_timestamp
);

create unique index uniq_substates_address on substates (address);

-- Event data
create table events
(
    id               integer   not NULL primary key AUTOINCREMENT,
    template_address text      not NULL,
    tx_hash          text      not NULL,
    topic            text      not NULL,
    -- The CBOR-encoded `Event`, served as stored.
    event            blob      not NULL,
    substate_id      text      NULL,
    -- The resource this event's substate_id names, set for the `std.resource.*` family only, so SSE
    -- consumers can filter a stream down to a single token's activity server-side.
    resource_address text      NULL,
    created_at       timestamp not null default current_timestamp,
    -- The epoch the transaction committed in. Retention ages events out by it.
    epoch            bigint    not null default 0
);

-- DB index for faster collection scan queries
create index events_indexer on events (template_address, tx_hash);
-- SSE event catch-up queries filter by topic, substate_id or resource_address and order by id.
create index events_topic_idx on events (topic);
create index events_substate_id_idx on events (substate_id);
create index events_resource_address_idx on events (resource_address);
create index events_epoch_idx on events (epoch);

-- Transaction receipts
create table transaction_receipts
(
    id              integer   not NULL primary key AUTOINCREMENT,
    address         text      not NULL,
    -- The CBOR-encoded `TransactionReceipt`, served as stored.
    data            blob      not NULL,
    created_at      timestamp not null default current_timestamp,
    outcome         text      not null default 'Commit',
    total_fees_paid bigint    not null default 0,
    -- The epoch the transaction committed in. Retention ages receipts and their events out by it.
    epoch           bigint    not null default 0
);

create unique index transaction_receipts_address_uniq on transaction_receipts (address);
create index transaction_receipts_epoch_idx on transaction_receipts (epoch);

create table transactions
(
    id              integer   not NULL primary key AUTOINCREMENT,
    transaction_id  text      not NULL,
    body            text      not null,
    created_at      timestamp not null default current_timestamp,
    rejected_reason text      null,
    rejected_at     timestamp null,
    -- The epoch at which the transaction reached a terminal state and became eligible for retention
    -- accounting: its commit epoch once a receipt is indexed, and until then its max_epoch, the last
    -- epoch in which it could still be sequenced.
    retention_epoch bigint    not null default 0,
    -- Where this indexer first learned of the transaction: 'local' for a direct submission through its
    -- API, 'gossip' for one observed on the network-wide transaction topic.
    source          text      not null default 'local'
);

create unique index transactions_transaction_id_uniq_idx on transactions (transaction_id);
create index transactions_retention_epoch_idx on transactions (retention_epoch);
-- `list_recent_transactions` pages backwards by id. Filtering by source without this index walks
-- back through the whole gossip stream to collect a page of local rows.
create index transactions_source_id_idx on transactions (source, id);

-- General purpose key value table
CREATE TABLE key_values
(
    id         INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    key        TEXT                              NOT NULL,
    value      TEXT                              NOT NULL,
    created_at DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX key_values_uniq_key on key_values (key);

-- Epoch checkpoints
CREATE TABLE epoch_checkpoints
(
    id          INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    epoch       BIGINT                            NOT NULL,
    shard_group TEXT                              NOT NULL,
    json_data   TEXT                              NOT NULL,
    created_at  DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- The shard range of the checkpoint's shard group, so that the checkpoints of the shard group containing a given
    -- shard can be selected. `shard_group` holds the range as "start-end".
    shard_start INTEGER                           NOT NULL DEFAULT 0,
    shard_end   INTEGER                           NOT NULL DEFAULT 0
);

CREATE UNIQUE INDEX epoch_checkpoints_uniq_epoch_shard_group ON epoch_checkpoints (epoch, shard_group);

create table utxos
(
    id               integer   not NULL primary key AUTOINCREMENT,
    commitment       text      not NULL,
    public_nonce     text      not NULL,
    version          bigint    not NULL,
    resource_address text      not NULL,
    shard            int       not NULL,
    state_version    bigint    not NULL,
    -- The CBOR-encoded `Utxo` while unspent, served as stored. NULL once spent.
    utxo             blob      NULL,
    utxo_tag         int       not NULL,
    epoch            bigint    not NULL,
    is_spent         boolean   not NULL,
    is_burnt         boolean   not NULL,
    created_at       timestamp not null default current_timestamp
);

-- Every read of `utxos` by state version is for one shard. With `shard` ahead of `state_version`, such a read is a
-- range within that shard's entries rather than over the whole resource's history.
CREATE INDEX utxos_resource_shard_state_version_epoch_idx ON utxos (resource_address, shard, state_version, epoch);
CREATE UNIQUE INDEX utxos_resource_public_nonce_utxo_tag_uniq_partial ON utxos (resource_address, public_nonce, utxo_tag) WHERE is_spent = false;
-- Finds a UTXO by its commitment, as the list cursor and marking an output spent do.
CREATE INDEX utxos_resource_commitment_idx ON utxos (resource_address, commitment);
-- A resource's unspent UTXOs in rowid order, so that listing them pages from a cursor without a sort. Queries
-- must repeat this condition as SQL literals for SQLite to use the index.
CREATE INDEX utxos_resource_unspent_idx ON utxos (resource_address) WHERE is_spent = false AND is_burnt = false;
-- Finds a resource's spent or burnt UTXOs by public nonce and tag, as fetching UTXOs by key does to report which of
-- the requested outputs are spent. Queries must repeat this condition as SQL literals for SQLite to use the index.
CREATE INDEX utxos_resource_public_nonce_utxo_tag_spent_idx ON utxos (resource_address, public_nonce, utxo_tag) WHERE is_spent = true OR is_burnt = true;

-- Lightweight template metadata received from validators via the TEMPLATE_METADATA sync flag. This
-- lets the indexer serve a searchable template catalogue without storing full WASM binaries.
CREATE TABLE template_catalogue
(
    id                INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    -- Hex-encoded template address (TemplateAddress = Hash32)
    template_address  TEXT                              NOT NULL UNIQUE,
    -- Human-readable template name extracted from the WASM ABI
    template_name     TEXT                              NOT NULL,
    -- Hex-encoded author public key (RistrettoPublicKeyBytes)
    author_public_key TEXT                              NOT NULL,
    -- Hex-encoded SHA-256 hash of the WASM binary
    binary_hash       TEXT                              NOT NULL,
    -- Epoch at which the template was published
    at_epoch          BIGINT                            NOT NULL,
    metadata_hash     TEXT                              NULL,
    created_at        TIMESTAMP                         NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at        TIMESTAMP                         NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX template_catalogue_template_name_idx ON template_catalogue (template_name);
CREATE INDEX template_catalogue_author_public_key_idx ON template_catalogue (author_public_key);
-- Used by the sync cursor filter (since_epoch) and ordering
CREATE INDEX template_catalogue_at_epoch_idx ON template_catalogue (at_epoch);

create table watched_substates
(
    id                integer   not null primary key autoincrement,
    component_address text      not null unique,
    template_address  text      not null,
    created_at        timestamp not null default current_timestamp
);

create index idx_watched_substates_template on watched_substates (template_address);

-- Committee-validated state merkle roots, recorded by the network state sync worker and consulted by
-- the read path to skip per-read commit-proof (QC chain) re-validation when a served root is already
-- trusted. The trust key is (epoch, shard_group, state_merkle_root); block_hash is diagnostics-only.
-- A small bounded ring of recent roots per (epoch, shard_group) is retained so reads landing on a
-- validator slightly behind the indexer's last probe still hit a trusted root.
create table verified_state_roots
(
    id                integer   not null primary key autoincrement,
    epoch             bigint    not null,
    shard_group       text      not null,
    block_height      bigint    not null,
    block_hash        text      not null,
    state_merkle_root text      not null,
    validated_at      timestamp not null default current_timestamp,
    unique (epoch, shard_group, state_merkle_root)
);

create index idx_verified_state_roots_lookup on verified_state_roots (epoch, shard_group, state_merkle_root);
create index idx_verified_state_roots_latest on verified_state_roots (epoch, shard_group, block_height);

-- Each substate's head version as this indexer last observed it, held here rather than in a side
-- store so that invalidation commits in the same transaction as the state sync watermark it is
-- derived from. An entry is served until a transition for its substate retires it, so the two must
-- never be visible out of step.
--
-- One row per substate, never per version. A live version is always the substate's head - upping a
-- substate downs its predecessor - so a cached head settles every lower version too: they are all
-- down, permanently, and are answered without consulting a validator or the sync watermark.
--
-- "This substate does not exist" is cached too, as a row with a null version. A `DoesNotExist`
-- lookup is settled by f+1 agreement, so it walks that many committee members every time; caching it
-- removes the most expensive lookup the indexer makes.
create table substate_cache
(
    substate_id     text    not null primary key,
    -- The substate's head version, or null when the substate does not exist.
    version         bigint  null,
    verified        boolean not null,
    substate_result blob    not null,
    cached_at       bigint  not null
);

create index idx_substate_cache_evict on substate_cache (cached_at);

-- Substates a synced transition has touched, retained only long enough to span a committee fetch.
-- A fetch that started before its shard reached `state_version` may have observed the substate as it
-- was beforehand, so a result landing afterwards must not be recorded as the head.
--
-- `substate_version` covers a fetch that starts afterwards and lands on a committee member that is
-- behind: that member answers with a version this indexer has already watched the substate pass, and
-- with no cached row left to rank against the answer would be installed as the head. The version is
-- a floor a result below it is refused by. It cannot also express that the floor version is down,
-- which a destroy with no successor leaves true, so `spent` carries that: a lagging member's `Up` at
-- the destroyed version must be refused while a `Down` at it is a legitimate head.
create table substate_cache_invalidations
(
    substate_id      text    not null primary key,
    state_version    bigint  not null,
    -- The substate version the transition showed: the version created, or the version destroyed.
    substate_version bigint  not null,
    -- Whether `substate_version` was destroyed rather than created.
    spent            boolean not null,
    invalidated_at   bigint  not null
);

create index idx_substate_cache_invalidations_expiry on substate_cache_invalidations (invalidated_at);

-- The proof each cached substate version was verified with, so that a request for it need not ask the
-- committee again. A proof is served only while its version is the head `substate_cache` holds, and
-- the pruner removes those that no longer match a cached head.
create table substate_cache_proofs
(
    substate_id  text   not null,
    version      bigint not null,
    -- CBOR-encoded SubstateValueProof
    value_proof  blob   not null,
    -- CBOR-encoded CommittedBlockProof anchoring `value_proof`
    commit_proof blob   not null,
    -- The epoch the substate value hash was computed in
    proof_epoch  bigint not null,
    primary key (substate_id, version)
);
