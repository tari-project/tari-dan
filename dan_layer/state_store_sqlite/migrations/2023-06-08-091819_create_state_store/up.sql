create table blocks
(
    id              integer   not null primary key AUTOINCREMENT,
    block_id        text      not NULL,
    parent_block_id text      not NULL,
    height          bigint    not NULL,
    leader_round    bigint    not NULL DEFAULT 0,
    epoch           bigint    not NULL,
    proposed_by     text      not NULL,
    justify         text      not NULL,
    prepared        text      not NULL,
    precommitted    text      not NULL,
    committed       text      not NULL,
    created_at      timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
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
    id                           integer   not NULL primary key AUTOINCREMENT,
    shard_id                     blob      not NULL,
    address                      text      not NULL,
    -- To be deleted in future
    version                      bigint    not NULL,
    data                         text      not NULL,
    created_by_payload_id        blob      not NULL,
    created_justify              text      NULL,
    created_node_hash            blob      not NULL,
    created_height               bigint    not NULL,
    destroyed_by_payload_id      blob      NULL,
    destroyed_justify            text      NULL,
    destroyed_node_hash          blob      NULL,
    destroyed_height             bigint    NULL,
    fee_paid_for_created_justify bigint    not NULL,
    fee_paid_for_deleted_justify bigint    not NULL,
    created_at_epoch             bigint    NULL,
    destroyed_at_epoch           bigint    NULL,
    created_justify_leader       text      NULL,
    destroyed_justify_leader     text      NULL,
    created_timestamp            timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    destroyed_timestamp          timestamp NULL
);

-- All shard ids are unique
create unique index uniq_substates_shard_id on substates (shard_id);

create table high_qcs
(
    id         integer   not null primary key autoincrement,
    epoch      bigint    not null,
    block_id   text      not null,
    height     bigint    not null,
    created_at timestamp NOT NULL default current_timestamp
);
create unique index high_qcs_idx_epoch_block_id_height on high_qcs (epoch, block_id, height);

create table transactions
(
    id                integer   not null primary key AUTOINCREMENT,
    transaction_id    text      not null,
    fee_instructions  text      not NULL,
    instructions      text      not NULL,
    sender_public_key text      not NULL,
    signature         text      not NULL,
    meta              text      not NULL,
    result            text      not NULL,
    involved_shards  text      not NULL,
    is_finalized      boolean   NOT NULL DEFAULT '0',
    created_at        timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

create unique index transactions_uniq_idx_id on transactions (transaction_id);

create table new_transaction_pool
(
    id             integer   not null primary key AUTOINCREMENT,
    transaction_id text      not null,
    decision       text      not null,
    fee            bigint    not null,
    created_at     timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id)
);
create unique index new_transaction_pool_uniq_idx_transaction_id on new_transaction_pool (transaction_id);

create table prepared_transaction_pool
(
    id             integer   not null primary key AUTOINCREMENT,
    transaction_id text      not null,
    decision       text      not null,
    fee            bigint    not null,
    is_ready       boolean   not null default '0',
    created_at     timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id)
);

create unique index prepared_transaction_pool_uniq_idx_transaction_id on prepared_transaction_pool (transaction_id);
-- fetching all by is_ready will be a very common operation
create index prepared_transaction_pool_idx_is_ready on prepared_transaction_pool (is_ready);

create table precommitted_transaction_pool
(
    id             integer   not null primary key AUTOINCREMENT,
    transaction_id text      not null,
    decision       text      not null,
    fee            bigint    not null,
    is_ready       boolean   not null default '0',
    created_at     timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id)
);

create unique index precommitted_transaction_pool_uniq_idx_transaction_id on precommitted_transaction_pool (transaction_id);
-- fetching all by is_ready will be a very common operation
create index precommitted_transaction_pool_idx_is_ready on precommitted_transaction_pool (is_ready);

create table committed_transaction_pool
(
    id             integer   not null primary key AUTOINCREMENT,
    transaction_id text      not null,
    decision       text      not null,
    fee            bigint    not null,
    is_ready       boolean   not null default '0',
    created_at     timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (transaction_id) REFERENCES transactions (transaction_id)
);

create unique index committed_transaction_pool_uniq_idx_transaction_id on committed_transaction_pool (transaction_id);
-- fetching all by is_ready will be a very common operation
create index committed_transaction_pool_idx_is_ready on committed_transaction_pool (is_ready);

