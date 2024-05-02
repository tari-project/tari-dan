create table quorum_certificates
(
    id         integer   not null primary key AUTOINCREMENT,
    qc_id      text      not NULL,
    block_id   text      not NULL,
    json       text      not NULL,
    created_at timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- fetching by qc_id is a very common operation
create unique index quorum_certificates_uniq_idx_id on quorum_certificates (qc_id);

create table blocks
(
    id                      integer   not null primary key AUTOINCREMENT,
    block_id                text      not NULL,
    parent_block_id         text      not NULL,
    merkle_root             text      not NULL,
    network                 text      not NULL,
    height                  bigint    not NULL,
    epoch                   bigint    not NULL,
    shard                   integer   not NULL,
    proposed_by             text      not NULL,
    qc_id                   text      not NULL,
    command_count           bigint    not NULL,
    commands                text      not NULL,
    total_leader_fee        bigint    not NULL,
    is_committed            boolean   not NULL default '0',
    is_processed            boolean   not NULL,
    is_dummy                boolean   not NULL,
    foreign_indexes         text      not NULL,
    signature               text      NULL,
    block_time              bigint    NULL,
    timestamp               bigint    not NULL,
    base_layer_block_height bigint    not NULL,
    base_layer_block_hash   text      not NULL,
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
    shard                   integer   not NULL,
    proposed_by             text      not NULL,
    justify                 text      not NULL,
    command_count           bigint    not NULL,
    commands                text      not NULL,
    total_leader_fee        bigint    not NULL,
    foreign_indexes         text      not NULL,
    signature               text      NULL,
    block_time              bigint    NULL,
    timestamp               bigint    not NULL,
    base_layer_block_height bigint    not NULL,
    base_layer_block_hash   text      not NULL,
    created_at              timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- block_id must be unique. Optimise fetching by block_id
create unique index parked_blocks_uniq_idx_id on parked_blocks (block_id);

create table leaf_blocks
(
    id           integer   not null primary key AUTOINCREMENT,
    block_id     text      not NULL,
    block_height bigint    not NULL,
    created_at   timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

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
    destroyed_by_transaction text      NULL,
    destroyed_justify        text      NULL,
    destroyed_by_block       text      NULL,
    created_at_epoch         bigint    not NULL,
    destroyed_at_epoch       bigint    NULL,
    read_locks               int       NOT NULL DEFAULT '0',
    is_locked_w              boolean   NOT NULL DEFAULT '0',
    locked_by                text      NULL,
    created_at               timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    destroyed_at             timestamp NULL
);

-- All shard ids are unique
create unique index substates_uniq_shard_id on substates (address);
-- querying for transaction ids that either Upd or Downd a substate
create index substates_idx_created_by_transaction on substates (created_by_transaction);
create index substates_idx_destroyed_by_transaction on substates (destroyed_by_transaction) where destroyed_by_transaction is not null;

create table high_qcs
(
    id           integer   not null primary key autoincrement,
    block_id     text      not null,
    block_height bigint    not null,
    qc_id        text      not null,
    created_at   timestamp NOT NULL default current_timestamp,
    FOREIGN KEY (qc_id) REFERENCES quorum_certificates (qc_id),
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

create unique index high_qcs_uniq_idx_qc_id on high_qcs (qc_id);

create table last_voted
(
    id         integer   not null primary key autoincrement,
    block_id   text      not null,
    height     bigint    not null,
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
    created_at timestamp NOT NULL default current_timestamp,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

create table last_proposed
(
    id         integer   not null primary key autoincrement,
    block_id   text      not null,
    height     bigint    not null,
    created_at timestamp NOT NULL default current_timestamp
);

create table locked_block
(
    id         integer   not null primary key autoincrement,
    block_id   text      not null,
    height     bigint    not null,
    created_at timestamp NOT NULL default current_timestamp,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

create table transactions
(
    id                integer   not null primary key AUTOINCREMENT,
    transaction_id    text      not null,
    fee_instructions  text      not NULL,
    instructions      text      not NULL,
    signature         text      not NULL,
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

create table transaction_pool
(
    id                  integer   not null primary key AUTOINCREMENT,
    transaction_id      text      not null,
    original_decision   text      not null,
    local_decision      text      null,
    remote_decision     text      null,
    evidence            text      not null,
    remote_evidence     text      null,
    transaction_fee     bigint    not null,
    leader_fee          bigint    null,
    global_exhaust_burn bigint    null,
    stage               text      not null,
    pending_stage       text      null,
    is_ready            boolean   not null,
    updated_at          timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at          timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id)
);
create unique index transaction_pool_uniq_idx_transaction_id on transaction_pool (transaction_id);
create index transaction_pool_idx_is_ready on transaction_pool (is_ready);

create table transaction_pool_state_updates
(
    id             integer   not null primary key AUTOINCREMENT,
    block_id       text      not null,
    block_height   bigint    not null,
    transaction_id text      not null,
    stage          text      not null,
    evidence       text      not null,
    is_ready       boolean   not null,
    local_decision text      not null,
    created_at     timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id),
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id)
);
create unique index transaction_pool_uniq_block_id_transaction_id on transaction_pool_state_updates (block_id, transaction_id);

create table locked_outputs
(
    id               integer   not null primary key AUTOINCREMENT,
    block_id         text      not null,
    transaction_id   text      not null,
    substate_address text      not null,
    created_at       timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id),
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);
create unique index locked_outputs_uniq_idx_substate_address on locked_outputs (substate_address);

create table votes
(
    id               integer   not null primary key AUTOINCREMENT,
    hash             text      not null,
    epoch            bigint    not null,
    block_id         text      not NULL,
    decision         integer   not null,
    signer_public_key       text      not null,
    signature        text      not NULL,
    created_at       timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);


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

CREATE TABLE foreign_proposals
(
    id                      integer   not NULL primary key AUTOINCREMENT,
    bucket                  int       not NULL,
    block_id                text      not NULL,
    state                   text      not NULL,
    proposed_height         bigint    NULL,
    transactions            text      not NULL,
    base_layer_block_height bigint    not NULL,
    created_at              timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (bucket, block_id)
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

CREATE TABLE state_tree
(
    id       integer not NULL primary key AUTOINCREMENT,
    key      text    not NULL,
    node     text    not NULL,
    is_stale boolean not null default '0'
);

-- Duplicate keys are not allowed
CREATE UNIQUE INDEX state_tree_uniq_idx_key on state_tree (key);
-- filtering out or by is_stale is used in every query
CREATE INDEX state_tree_idx_is_stale on state_tree (is_stale);

CREATE TABLE pending_state_tree_diffs
(
    id           integer   not NULL primary key AUTOINCREMENT,
    block_id     text      not NULL,
    block_height bigint    not NULL,
    diff_json    text      not NULL,
    created_at   timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (block_id) REFERENCES blocks (block_id)
);

CREATE UNIQUE INDEX pending_state_tree_diffs_uniq_idx_block_id on pending_state_tree_diffs (block_id);


-- Debug Triggers
CREATE TABLE transaction_pool_history
(
    history_id          INTEGER PRIMARY KEY,
    id                  integer   not null,
    transaction_id      text      not null,
    original_decision   text      not null,
    local_decision      text      null,
    remote_decision     text      null,
    evidence            text      not null,
    transaction_fee     bigint    not null,
    leader_fee          bigint    null,
    global_exhaust_burn bigint    null,
    stage               text      not null,
    new_stage           text      not null,
    is_ready            boolean   not null,
    new_is_ready        boolean   not null,
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
                                          transaction_fee,
                                          leader_fee,
                                          global_exhaust_burn,
                                          stage,
                                          new_stage,
                                          is_ready,
                                          new_is_ready,
                                          updated_at,
                                          created_at)
    VALUES (OLD.id,
            OLD.transaction_id,
            OLD.original_decision,
            OLD.local_decision,
            OLD.remote_decision,
            OLD.evidence,
            OLD.transaction_fee,
            OLD.leader_fee,
            OLD.global_exhaust_burn,
            OLD.stage,
            NEW.stage,
            OLD.is_ready,
            NEW.is_ready,
            OLD.updated_at,
            OLD.created_at);
END;
