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

-- fetching by the payload_id will be a very common operation
create index payload_index_payload_id on payloads (payload_id);


create table votes (
    id integer not null primary key AUTOINCREMENT,
    tree_node_hash blob not NULL,
    shard_id blob not NULL,
    address blob not NULL,
    node_height bigint not NULL,
    vote_message text not NULL
);

-- fetching by node_height will be a very common operation
create index votes_index_node_height on votes (node_height);
-- fetching by the pair (tree_node_hash, shard_id) will be a very common operation
create index votes_index_tree_node_hash_shard_id on votes (tree_node_hash, shard_id);
-- fetching by the triplet (tree_node_hash, shard_id, address) will be a very common operation
create index votes_index_tree_node_hash_shard_id_address on votes (tree_node_hash, shard_id, address);


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
create index nodes_index_node_hash on nodes (node_hash);


create table last_executed_heights (
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    node_height bigint not NULL
);

-- fetching by shard_id will be a very common operation
create index last_executed_height_index_shard_id on last_executed_heights (shard_id);


create table payload_votes (
    id integer not null primary key AUTOINCREMENT,
    payload_id blob not NULL,
    shard_id blob not NULL,
    node_height bigint not NULL,
    hotstuff_tree_node text not NULL
);

-- fetching by (payload_id, shard_id, node_height) will be a very common operation
create index payload_votes_index_payload_id_shard_id_node_height on payload_votes (payload_id, shard_id, node_height);


create table objects (
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    payload_id blob not NULL,
    object_id blob not NULL,
    node_height bigint not NULL,
    substate_change text not NULL,

    object_pledge text not NULL -- TODO: can it be non null ?
);

-- fetching by (shard_id, object_id) will be a very common operation
create index objects_index_shard_id_object_id_node_height_substate_change on objects (shard_id, object_id, payload_id, node_height, substate_change);


create table substate_changes (
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    substate_change text not null,
    tree_node_hash blob not NULL
);

-- fetching by (shard_id, tree_node_hash) will be a very common operation
create index substate_changes_shard_id_tree_node_hash on substate_changes (shard_id, tree_node_hash);
