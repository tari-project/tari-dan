create table payloads (
    id integer not null primary key AUTOINCREMENT,
    payload_id blob not null,
    instructions text not NULL,
    public_nonce blob not NULL,
    scalar blob not NULL,
    fee bigint not NULL,
    sender_public_key blob not NULL,
    meta text NOT NULL
);

create unique index payload_index_payload_id on payloads (payload_id) ;


create table received_votes (
    id integer not null primary key AUTOINCREMENT,
    tree_node_hash blob not NULL,
    shard_id blob not NULL,
    address blob not NULL,
    vote_message text not NULL
);

create table leaf_nodes (
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    tree_node_hash blob not NULL,
    node_height bigint not NULL
);

-- fetching by shard_id will be a very common operation
create index leaf_nodes_index_shard_id on leaf_nodes (shard_id);


create table last_voted_heights (
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    node_height bigint not NULL
);

-- fetching by shard_id will be a very common operation
create index last_voted_height_index_shard_id on last_voted_heights (shard_id);


create table lock_node_and_heights (
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    tree_node_hash blob not NULL,
    node_height bigint not NULL
);

-- fetching by shard_id will be a very common operation
create index lock_node_and_heights_index_shard_id on lock_node_and_heights (shard_id);

drop table nodes;
create table nodes (
    id integer not null primary key AUTOINCREMENT,
    node_hash blob not NULL,
    parent_node_hash blob not NULL,
    height bigint not NULL,
    shard blob not NULL,
    payload_id blob not NULL,
    payload_height bigint not NULL,
    local_pledges text not NULL,
    epoch bigint not NULL,
    proposed_by blob not NULL,
    justify text not NULL
);

-- fetching by tree_node_hash will be a very common operation
create unique index nodes_index_node_hash on nodes (node_hash);


create table last_executed_heights (
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    node_height bigint not NULL
);

-- fetching by shard_id will be a very common operation
create index last_executed_height_index_shard_id on last_executed_heights (shard_id);


create table leader_proposals (
    id integer not null primary key AUTOINCREMENT,
    payload_id blob not NULL,
    shard_id blob not NULL,
    payload_height bigint not NULL,
    node_hash blob not NULL,
    hotstuff_tree_node text not NULL
);

create unique index leader_proposals_index on leader_proposals (payload_id, shard_id, payload_height, node_hash);

create table objects (
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    payload_id blob not NULL,
    object_id blob not NULL,
    node_height bigint not NULL,
    current_state text not NULL,
    object_pledge text not NULL -- TODO: can it be non null ?
);


create table substate_changes (
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    substate_change text not null,
    tree_node_hash blob not NULL
);

