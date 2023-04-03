create table payloads
(
    id               integer   not null primary key AUTOINCREMENT,
    payload_id       blob      not null,
    fee_instructions text      not NULL,
    instructions     text      not NULL,
    public_nonce     blob      not NULL,
    scalar           blob      not NULL,
    sender_address   blob      not NULL,
    meta             text      not NULL,
    result           text      NULL,
    timestamp        timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

create unique index payload_index_payload_id on payloads (payload_id);

create table received_votes
(
    id             integer   not null primary key AUTOINCREMENT,
    tree_node_hash blob      not NULL,
    address        blob      not NULL,
    vote_message   text      not NULL,
    timestamp      timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

create table leaf_nodes
(
    id             integer   not null primary key AUTOINCREMENT,
    shard_id       blob      not NULL,
    payload_id     blob      not NULL,
    payload_height bigint    not NULL,
    tree_node_hash blob      not NULL,
    node_height    bigint    not NULL,
    timestamp      timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- fetching by <payload_id, shard_id> will be a very common operation
create index leaf_nodes_index_payload_id_shard_id on leaf_nodes (payload_id, shard_id);


create table last_voted_heights
(
    id           integer   not null primary key AUTOINCREMENT,
    payload_id   blob      not NULL,
    shard_id     blob      not NULL,
    node_height  bigint    not NULL,
    leader_round bigint             DEFAULT 0 NOT NULL,
    timestamp    timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- fetching by shard_id will be a very common operation
create index last_voted_height_index_shard_id on last_voted_heights (shard_id);


create table lock_node_and_heights
(
    id             integer   not null primary key AUTOINCREMENT,
    payload_id     blob      not NULL,
    shard_id       blob      not NULL,
    tree_node_hash blob      not NULL,
    node_height    bigint    not NULL,
    timestamp      timestamp not NULL DEFAULT CURRENT_TIMESTAMP
);

-- fetching by shard_id, payload_id will be a very common operation
create index lock_node_and_heights_index_shard_id_payload_id on lock_node_and_heights (shard_id, payload_id);

create table nodes
(
    id               integer   not null primary key AUTOINCREMENT,
    node_hash        blob      not NULL,
    parent_node_hash blob      not NULL,
    height           bigint    not NULL,
    shard            blob      not NULL,
    payload_id       blob      not NULL,
    payload_height   bigint    not NULL,
    leader_round     bigint    not NULL DEFAULT 0,
    local_pledges    text      not NULL,
    epoch            bigint    not NULL,
    proposed_by      blob      not NULL,
    justify          text      not NULL,
    timestamp        timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- fetching by tree_node_hash will be a very common operation
create unique index nodes_index_node_hash on nodes (node_hash);

create table last_executed_heights
(
    id          integer   not null primary key AUTOINCREMENT,
    payload_id  blob      not NULL,
    shard_id    blob      not NULL,
    node_height bigint    not NULL,
    timestamp   timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- fetching by shard_id will be a very common operation
create index last_executed_height_index_shard_id on last_executed_heights (shard_id);


create table leader_proposals
(
    id                 integer   not null primary key AUTOINCREMENT,
    payload_id         blob      not NULL,
    shard_id           blob      not NULL,
    payload_height     bigint    not NULL,
    leader_round       bigint    not NULL DEFAULT 0,
    node_hash          blob      not NULL,
    hotstuff_tree_node text      not NULL,
    timestamp          timestamp not NULL default current_timestamp
);

create unique index leader_proposals_index on leader_proposals (payload_id, shard_id, payload_height, node_hash);

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
    created_timestamp            timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    destroyed_timestamp          timestamp NULL,
    fee_paid_for_created_justify bigint    not NULL,
    fee_paid_for_deleted_justify bigint    not NULL,
    created_at_epoch             bigint    NULL,
    deleted_at_epoch             bigint    NULL,
    created_justify_leader       text      NULL,
    deleted_justify_leader       text      NULL
);

-- All shard ids are unique
create unique index uniq_substates_shard_id on substates (shard_id);

create table shard_pledges
(
    id                          integer   not NULL primary key AUTOINCREMENT,
    shard_id                    blob      not NULL,
    created_height              bigint    not NULL,
    pledged_to_payload_id       blob      not NULL,
    is_active                   boolean   not NULL,
    completed_by_tree_node_hash blob      NULL,
    abandoned_by_tree_node_hash blob      NULL,
    timestamp                   timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_timestamp           timestamp NULL
);

create table high_qcs
(
    id         integer   not null primary key autoincrement,
    shard_id   blob      not null,
    payload_id blob      not null,
    height     bigint    not null,
    qc_json    text      not null,
    identity   blob      not null,
    timestamp  timestamp NOT NULL default current_timestamp
);
create unique index high_qcs_index_shard_id_height on high_qcs (shard_id, payload_id, height);

CREATE TABLE current_leader_states
(
    id           integer   NOT NULL PRIMARY KEY AUTOINCREMENT,
    payload_id   blob      NOT NULL,
    shard_id     blob      NOT NULL,
    leader_round bigint    NOT NULL,
    leader       blob      NOT NULL,
    timestamp    timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX current_leader_states_index_payload_id_shard_id ON current_leader_states (payload_id, shard_id);
