create table quorum_certificates
(
    id          integer   not null primary key AUTOINCREMENT,
    qc_id       text      not NULL,
    block_id    text      not NULL,
    shard_group integer   not NULL,
    json        text      not NULL,
    created_at  timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- fetching by qc_id is a very common operation
create unique index quorum_certificates_uniq_idx_id on quorum_certificates (qc_id);

create table blocks
(
    id                      integer   not null primary key AUTOINCREMENT,
    block_id                text      not NULL,
    parent_block_id         text      not NULL REFERENCES blocks (block_id),
    merkle_root             text      not NULL,
    network                 text      not NULL,
    height                  bigint    not NULL,
    epoch                   bigint    not NULL,
    shard_group             integer   not NULL,
    proposed_by             text      not NULL,
    qc_id                   text      not NULL,
    command_count           bigint    not NULL,
    commands                text      not NULL,
    total_leader_fee        bigint    not NULL,
    is_committed            boolean   not NULL default '0',
    is_justified            boolean   not NULL,
    is_dummy                boolean   not NULL,
    foreign_indexes         text      not NULL,
    signature               text      NULL,
    block_time              bigint    NULL,
    timestamp               bigint    not NULL,
    base_layer_block_height bigint    not NULL,
    base_layer_block_hash   text      not NULL,
    extra_data              text      NULL,
    created_at              timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (qc_id) REFERENCES quorum_certificates (qc_id)
);

-- block_id must be unique. Optimise fetching by block_id
create unique index blocks_uniq_idx_id on blocks (block_id);

create table parked_blocks
(
    id                      integer   not null primary key AUTOINCREMENT,
    block_id                text      not NULL,
    parent_block_id         text      not NULL,
    merkle_root             text      not NULL,
    network                 text      not NULL,
    height                  bigint    not NULL,
    epoch                   bigint    not NULL,
    shard_group             integer   not NULL,
    proposed_by             text      not NULL,
    justify                 text      not NULL,
    command_count           bigint    not NULL,
    commands                text      not NULL,
    total_leader_fee        bigint    not NULL,
    foreign_indexes         text      not NULL,
    signature               text      NULL,
    timestamp               bigint    not NULL,
    base_layer_block_height bigint    not NULL,
    base_layer_block_hash   text      not NULL,
    foreign_proposals       text      not NULL,
    extra_data              text      NULL,
    created_at              timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- block_id must be unique. Optimise fetching by block_id
create unique index parked_blocks_uniq_idx_id on parked_blocks (block_id);

CREATE TABLE missing_transactions
(
    id                    integer   not NULL primary key AUTOINCREMENT,
    block_id              text      not NULL,
    block_height          bigint    not NULL,
    transaction_id        text      not NULL,
    is_awaiting_execution boolean   not NULL,
    created_at            timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (block_id) REFERENCES parked_blocks (block_id)
);

create table foreign_parked_blocks
(
    id            integer   not null primary key AUTOINCREMENT,
    block_id      text      not NULL,
    block         text      not NULL,
    block_pledges text      not NULL,
    justify_qc    text      not NULL,
    created_at    timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- block_id must be unique. Optimise fetching by block_id
create unique index foreign_parked_blocks_uniq_idx_id on parked_blocks (block_id);

CREATE TABLE foreign_missing_transactions
(
    id              integer   not NULL primary key AUTOINCREMENT,
    parked_block_id integer   not NULL,
    transaction_id  text      not NULL,
    created_at      timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (parked_block_id) REFERENCES foreign_parked_blocks (id)
);

create table leaf_blocks
(
    id           integer   not null primary key AUTOINCREMENT,
    block_id     text      not NULL,
    block_height bigint    not NULL,
    epoch        bigint    not NULL,
    created_at   timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

create table block_diffs
(
    id             integer   NOT NULL primary key AUTOINCREMENT,
    block_id       text      NOT NULL,
    transaction_id text      NOT NULL,
    substate_id    text      NOT NULL,
    version        int       NOT NULL,
    shard          int       NOT NULL,
    -- Up or Down
    change         text      NOT NULL,
    -- NULL for Down
    state          text      NULL,
    created_at     timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
--    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id),
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);
create index block_diffs_idx_block_id_substate_id on block_diffs (block_id, substate_id);

create table substates
(
    id                       integer   not NULL primary key AUTOINCREMENT,
    address                  text      not NULL,
    substate_id              text      not NULL,
    version                  integer   not NULL,
    data                     text      not NULL,
    state_hash               text      not NULL,
    created_by_transaction   text      not NULL,
    created_justify          text      not NULL,
    created_block            text      not NULL,
    created_height           bigint    not NULL,
    -- <epoch, shard> uniquely identifies the chain
    created_at_epoch         bigint    not NULL,
    created_by_shard         int       not NULL,
    destroyed_by_transaction text      NULL,
    destroyed_justify        text      NULL,
    destroyed_by_block       bigint    NULL,
    -- <epoch, shard> uniquely identifies the chain
    destroyed_at_epoch       bigint    NULL,
    destroyed_by_shard       int       NULL,
    created_at               timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    destroyed_at             timestamp NULL
);

-- All addresses are unique
create unique index substates_uniq_address on substates (address);
-- All substate_id, version pairs are unique. This is a common query
create unique index substates_uniq_substate_id_and_version on substates (substate_id, version);
-- querying for transaction ids that either Upd or Downd a substate
create index substates_idx_created_by_transaction on substates (created_by_transaction);
create index substates_idx_destroyed_by_transaction on substates (destroyed_by_transaction) where destroyed_by_transaction is not null;

create table foreign_substate_pledges
(
    id             integer   NOT NULL primary key AUTOINCREMENT,
    transaction_id text      NOT NULL,
    address        text      NOT NULL,
    substate_id    text      NOT NULL,
    version        int       NOT NULL,
    substate_value text      NULL,
    shard_group    int       NOT NULL,
    lock_type      text      NOT NULL,
    created_at     timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id),
    CHECK (lock_type IN ('Write', 'Read', 'Output'))
);

create index foreign_substate_pledges_transaction_id_idx on foreign_substate_pledges (transaction_id);
create unique index foreign_substate_pledges_transaction_id_substate_id_uniq_idx on foreign_substate_pledges (transaction_id, substate_id);

create table substate_locks
(
    id             integer   NOT NULL primary key AUTOINCREMENT,
    block_id       text      NOT NULL,
    transaction_id text      NOT NULL,
    substate_id    text      NOT NULL,
    version        int       NOT NULL,
    -- Write, Read or Output
    lock           text      NOT NULL,
    is_local_only  boolean   NOT NULL DEFAULT '0',
    created_at     timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id),
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

create table high_qcs
(
    id           integer   not null primary key autoincrement,
    block_id     text      not null,
    block_height bigint    not null,
    epoch        bigint    not null,
    qc_id        text      not null,
    created_at   timestamp NOT NULL default current_timestamp,
    FOREIGN KEY (qc_id) REFERENCES quorum_certificates (qc_id),
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

create table last_voted
(
    id         integer   not null primary key autoincrement,
    block_id   text      not null,
    height     bigint    not null,
    epoch      bigint    not null,
    created_at timestamp NOT NULL default current_timestamp
);

create table last_sent_vote
(
    id           integer   NOT NULL PRIMARY KEY AUTOINCREMENT,
    epoch        bigint    NOT NULL,
    block_id     text      NOT NULL,
    block_height bigint    NOT NULL,
    decision     integer   NOT NULL,
    signature    text      NOT NULL,
    created_at   timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

create table last_executed
(
    id         integer   not null primary key autoincrement,
    block_id   text      not null,
    height     bigint    not null,
    epoch      bigint    not null,
    created_at timestamp NOT NULL default current_timestamp,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

create table last_proposed
(
    id         integer   not null primary key autoincrement,
    block_id   text      not null,
    height     bigint    not null,
    epoch      bigint    not null,
    created_at timestamp NOT NULL default current_timestamp
);

create table locked_block
(
    id         integer   not null primary key autoincrement,
    block_id   text      not null,
    height     bigint    not null,
    epoch      bigint    not null,
    created_at timestamp NOT NULL default current_timestamp,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

create table transactions
(
    id                integer   not null primary key AUTOINCREMENT,
    transaction_id    text      not null,
    fee_instructions  text      not NULL,
    instructions      text      not NULL,
    signatures        text      not NULL,
    inputs            text      not NULL,
    filled_inputs     text      not NULL,
    resolved_inputs   text      NULL,
    resulting_outputs text      NULL,
    result            text      NULL,
    execution_time_ms bigint    NULL,
    final_decision    text      NULL,
    finalized_at      timestamp NULL,
    abort_details     text      NULL,
    min_epoch         BIGINT    NULL,
    max_epoch         BIGINT    NULL,
    created_at        timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

create unique index transactions_uniq_idx_id on transactions (transaction_id);

create table transaction_executions
(
    id                integer   NOT NULL primary key AUTOINCREMENT,
    -- Note: the block_id may not be in the database if the block is being proposed ∴ no foreign key
    block_id          text      NOT NULL,
    transaction_id    text      NOT NULL,
    resolved_inputs   text      NOT NULL,
    resulting_outputs text      NOT NULL,
    result            text      NOT NULL,
    execution_time_ms bigint    NOT NULL,
    abort_reason      text      NULL,
    created_at        timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id)
);

create unique index transaction_executions_uniq_block_id_transaction_id on transaction_executions (block_id, transaction_id);

create table transaction_pool
(
    id                integer   not null primary key AUTOINCREMENT,
    transaction_id    text      not null,
    original_decision text      not null,
    local_decision    text      null,
    remote_decision   text      null,
    evidence          text      null,
    transaction_fee   bigint    not null DEFAULT 0,
    leader_fee        text      null,
    stage             text      not null,
    pending_stage     text      null,
    is_ready          boolean   not null,
    confirm_stage     text      null,
    updated_at        timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at        timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id)
);
create unique index transaction_pool_uniq_idx_transaction_id on transaction_pool (transaction_id);
create index transaction_pool_idx_is_ready on transaction_pool (is_ready);

create table transaction_pool_state_updates
(
    id              integer   not null primary key AUTOINCREMENT,
    block_id        text      not null,
    block_height    bigint    not null,
    transaction_id  text      not null,
    stage           text      not null,
    evidence        text      not null,
    is_ready        boolean   not null,
    local_decision  text      not null,
    transaction_fee bigint    not null,
    leader_fee      text      null,
    remote_decision text      null,
    is_applied      boolean   not null DEFAULT '0',
    created_at      timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id),
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id)
);
create unique index transaction_pool_state_updates_uniq_block_id_transaction_id on transaction_pool_state_updates (block_id, transaction_id);
create index transaction_pool_state_updates_idx_is_applied on transaction_pool_state_updates (is_applied);

create table votes
(
    id               integer   not null primary key AUTOINCREMENT,
    hash             text      not null,
    epoch            bigint    not null,
    block_id         text      not NULL,
    decision         integer   not null,
    sender_leaf_hash text      not NULL,
    signature        text      not NULL,
    created_at       timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE foreign_proposals
(
    id                       integer   not null primary key AUTOINCREMENT,
    block_id                 text      not NULL,
    parent_block_id          text      not NULL,
    merkle_root              text      not NULL,
    network                  text      not NULL,
    height                   bigint    not NULL,
    epoch                    bigint    not NULL,
    shard_group              integer   not NULL,
    proposed_by              text      not NULL,
    qc                       text      not NULL,
    command_count            bigint    not NULL,
    commands                 text      not NULL,
    total_leader_fee         bigint    not NULL,
    foreign_indexes          text      not NULL,
    signature                text      NULL,
    timestamp                bigint    not NULL,
    base_layer_block_height  bigint    not NULL,
    base_layer_block_hash    text      not NULL,
    justify_qc_id            text      not NULL REFERENCES quorum_certificates (qc_id),
    block_pledge             text      not NULL,
    proposed_in_block        text      NULL REFERENCES blocks (block_id),
    proposed_in_block_height bigint    NULL,
    status                   text      not NULL,
    extra_data               text      NULL,
    created_at               timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (block_id)
);

CREATE TABLE foreign_send_counters
(
    id         integer   not NULL primary key AUTOINCREMENT,
    block_id   text      not NULL,
    counters   text      not NULL,
    created_at timestamp not NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE foreign_receive_counters
(
    id         integer   not NULL primary key AUTOINCREMENT,
    counters   text      not NULL,
    created_at timestamp not NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE burnt_utxos
(
    id                       integer   not null primary key AUTOINCREMENT,
    substate_id              text      not NULL,
    substate                 text      not NULL,
    base_layer_block_height  bigint    not NULL,
    proposed_in_block        text      NULL REFERENCES blocks (block_id),
    proposed_in_block_height bigint    NULL,
    created_at               timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (substate_id)
);

CREATE TABLE state_tree
(
    id       integer not NULL primary key AUTOINCREMENT,
    shard    int     not NULL,
    key      text    not NULL,
    node     text    not NULL,
    is_stale boolean not null default '0'
);

-- Scoping by shard
CREATE INDEX state_tree_idx_shard_key on state_tree (shard) WHERE is_stale = false;
-- Duplicate keys are not allowed
CREATE UNIQUE INDEX state_tree_uniq_idx_key on state_tree (shard, key) WHERE is_stale = false;
-- filtering out or by is_stale is used in every query
CREATE INDEX state_tree_idx_is_stale on state_tree (is_stale);

create table state_tree_shard_versions
(
    id         integer   not null primary key AUTOINCREMENT,
    shard      integer   not NULL,
    version    bigint    not NULL,
    created_at timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- One entry per shard
CREATE UNIQUE INDEX state_tree_uniq_shard_versions_shard on state_tree_shard_versions (shard);

CREATE TABLE pending_state_tree_diffs
(
    id           integer   not NULL primary key AUTOINCREMENT,
    block_id     text      not NULL,
    block_height bigint    not NULL,
    shard        integer   not NULL,
    version      bigint    not NULL,
    diff_json    text      not NULL,
    created_at   timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

CREATE UNIQUE INDEX pending_state_tree_diffs_uniq_idx_block_id_shard on pending_state_tree_diffs (block_id, shard);

CREATE TABLE epoch_checkpoints
(
    id           integer   not NULL primary key AUTOINCREMENT,
    epoch        bigint    not NULL,
    commit_block text      not NULL,
    qcs          text      not NULL,
    shard_roots  text      not NULL,
    created_at   timestamp not NULL DEFAULT CURRENT_TIMESTAMP
);

-- An append-only store of state transitions
CREATE TABLE state_transitions
(
    id               integer                                   not NULL primary key AUTOINCREMENT,
    -- <epoch, shard> tuple uniquely identifies the "chain" that created the state transition
    epoch            bigint                                    not NULL,
    shard            int                                       not NULL,
    -- in conjunction with the <epoch, shard>, this uniquely identifies and orders the state transition
    seq              bigint                                    not NULL,
    substate_address text                                      not NULL,
    -- substate_id and version not required, just to make DB inspection easier
    substate_id      text                                      not NULL,
    version          int                                       not NULL,
    transition       text check (transition IN ('UP', 'DOWN')) not NULL,
    state_hash       text                                      NULL,
    state_version    bigint                                    not NULL,
    created_at       timestamp                                 not NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (substate_address) REFERENCES substates (address)
);
CREATE UNIQUE INDEX state_transitions_shard_seq on state_transitions (shard, seq);
CREATE INDEX state_transitions_epoch on state_transitions (epoch);

-- Debug Triggers
CREATE TABLE transaction_pool_history
(
    history_id          INTEGER PRIMARY KEY,
    id                  integer   not null,
    transaction_id      text      not null,
    original_decision   text      not null,
    local_decision      text      null,
    remote_decision     text      null,
    evidence            text      null,
    new_evidence        text      null,
    transaction_fee     bigint    null,
    leader_fee          bigint    null,
    global_exhaust_burn bigint    null,
    stage               text      not null,
    new_stage           text      not null,
    pending_stage       text      null,
    new_pending_stage   text      null,
    is_ready            boolean   not null,
    new_is_ready        boolean   not null,
    confirm_stage       text      null,
    new_confirm_stage   text      null,
    updated_at          timestamp NOT NULL,
    created_at          timestamp NOT NULL,
    change_time         DATETIME DEFAULT (STRFTIME('%Y-%m-%d %H:%M:%f', 'NOW'))
);

CREATE TRIGGER copy_transaction_pool_history
    AFTER UPDATE
    ON transaction_pool
    FOR EACH ROW
BEGIN
    INSERT INTO transaction_pool_history (id,
                                          transaction_id,
                                          original_decision,
                                          local_decision,
                                          remote_decision,
                                          evidence,
                                          new_evidence,
                                          transaction_fee,
                                          leader_fee,
                                          stage,
                                          new_stage,
                                          pending_stage,
                                          new_pending_stage,
                                          is_ready,
                                          new_is_ready,
                                          confirm_stage,
                                          new_confirm_stage,
                                          updated_at,
                                          created_at)
    VALUES (OLD.id,
            OLD.transaction_id,
            OLD.original_decision,
            OLD.local_decision,
            OLD.remote_decision,
            OLD.evidence,
            NEW.evidence,
            NEW.transaction_fee,
            NEW.leader_fee,
            OLD.stage,
            NEW.stage,
            OLD.pending_stage,
            NEW.pending_stage,
            OLD.is_ready,
            NEW.is_ready,
            OLD.confirm_stage,
            NEW.confirm_stage,
            OLD.updated_at,
            OLD.created_at);
END;
