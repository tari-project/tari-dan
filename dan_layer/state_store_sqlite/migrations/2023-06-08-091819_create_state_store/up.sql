create table quorum_certificates
(
    id         integer   not null primary key AUTOINCREMENT,
    qc_id      text      not NULL,
    json       text      not NULL,
    created_at timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- fetching by qc_id will be a very common operation
create unique index quorum_certificates_uniq_idx_id on quorum_certificates (qc_id);

create table blocks
(
    id              integer   not null primary key AUTOINCREMENT,
    block_id        text      not NULL,
    parent_block_id text      not NULL,
    height          bigint    not NULL,
    leader_round    bigint    not NULL DEFAULT 0,
    epoch           bigint    not NULL,
    proposed_by     text      not NULL,
    qc_id           text      not NULL,
    commands        text      not NULL,
    created_at      timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (qc_id) REFERENCES quorum_certificates (qc_id)
);

-- fetching by block_id will be a very common operation
create unique index blocks_uniq_idx_id on blocks (block_id);

create table leaf_blocks
(
    id           integer   not null primary key AUTOINCREMENT,
    epoch        bigint    not NULL,
    block_id     text      not NULL,
    block_height bigint    not NULL,
    created_at   timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

create table substates
(
    id                       integer   not NULL primary key AUTOINCREMENT,
    shard_id                 text      not NULL,
    address                  text      not NULL,
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
create unique index substates_uniq_shard_id on substates (shard_id);

create table high_qcs
(
    id         integer   not null primary key autoincrement,
    epoch      bigint    not NULL,
    block_id   text      not null,
    qc_id      text      not null,
    created_at timestamp NOT NULL default current_timestamp,
    FOREIGN KEY (qc_id) REFERENCES quorum_certificates (qc_id)
);

create unique index high_qcs_uniq_idx_qc_id on high_qcs (qc_id);

create table last_voted
(
    id         integer   not null primary key autoincrement,
    epoch      bigint    not null,
    block_id   text      not null,
    height     bigint    not null,
    created_at timestamp NOT NULL default current_timestamp
);

create table last_executed
(
    id         integer   not null primary key autoincrement,
    epoch      bigint    not null,
    block_id   text      not null,
    height     bigint    not null,
    created_at timestamp NOT NULL default current_timestamp
);

create table last_proposed
(
    id         integer   not null primary key autoincrement,
    epoch      bigint    not null,
    block_id   text      not null,
    height     bigint    not null,
    created_at timestamp NOT NULL default current_timestamp
);

create table locked_block
(
    id         integer   not null primary key autoincrement,
    epoch      bigint    not null,
    block_id   text      not null,
    height     bigint    not null,
    created_at timestamp NOT NULL default current_timestamp
);

create table transactions
(
    id                integer   not null primary key AUTOINCREMENT,
    transaction_id    text      not null,
    fee_instructions  text      not NULL,
    instructions      text      not NULL,
    signature         text      not NULL,
    inputs            text      not NULL,
    input_refs        text      not NULL,
    outputs           text      not NULL,
    filled_inputs     text      not NULL,
    filled_outputs    text      not NULL,
    result            text      NULL,
    execution_time_ms bigint    NULL,
    final_decision   text      NULL,
    created_at        timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

create unique index transactions_uniq_idx_id on transactions (transaction_id);

create table transaction_pool
(
    id                integer   not null primary key AUTOINCREMENT,
    transaction_id    text      not null,
    involved_shards   text      not null,
    original_decision text      not null,
    pending_decision  text      null,
    evidence          text      not null,
    fee               bigint    not null,
    stage             text      not null,
    is_ready          boolean   not null,
    updated_at        timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at        timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id)
);
create unique index transaction_pool_uniq_idx_transaction_id on transaction_pool (transaction_id);
create index transaction_pool_idx_is_ready on transaction_pool (is_ready);

create table votes
(
    id               integer   not null primary key AUTOINCREMENT,
    hash             text      not null,
    epoch            bigint    not null,
    block_id         text      not NULL,
    decision         integer   not null,
    sender_leaf_hash text      not NULL,
    signature        text      not NULL,
    merkle_proof     text      not NULL,
    created_at       timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE block_missing_txs
(
    id              integer   not NULL PRIMARY KEY AUTOINCREMENT,
    transaction_ids text      not NULL,
    block_id        text      not NULL,
    created_at      timestamp not NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE missing_tx
(
    id              integer   not NULL primary key AUTOINCREMENT,
    transaction_id  text      not NULL,
    block_id        text      not NULL,
    created_at      timestamp not NULL DEFAULT CURRENT_TIMESTAMP
);
