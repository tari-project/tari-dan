create table payload_table {
    id integer not null primary key AUTOINCREMENT
    payload_id blob not null,
    instructions blob not NULL,
    public_nonce blob not NULL,
    scalar blob not NULL,
    fee integer not NULL,
    sender_public_key blob not NULL,
    meta blob
}

-- fetching by the payload_id will be a very common operation
create index payload_index on payload (payload_id);


create table votes {
    id integer not null primary key AUTOINCREMENT,
    tree_node_hash blob not NULL,
    shard_id blob not NULL,
    address blob not NULL,
    node_height integer not NULL,
    vote_message blob not NULL
}

-- fetching by the pair (tree_node_hash, shard_id) will be a very common operation
create index votes_index on votes (tree_node_hash, shard_id, node_height)


create table leaf_nodes {
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    tree_node_hash blob not NULL,
    node_height integer not NULL
}

-- fetching by shard_id will be a very common operation
create index leaf_nodes_index on leaf_nodes (shard_id)


create table last_voted_heights {
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    node_height integer not NULL
}

-- fetching by shard_id will be a very common operation
create index last_voted_height_index on last_voted_height (shard_id)


create table lock_node_and_heights {
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    tree_node_hash blob not NULL,
    node_height integer not NULL
}

-- fetching by shard_id will be a very common operation
create index lock_node_and_heights_index on lock_node_and_heights (shard_id)


create table nodes {
    id integer not null primary key AUTOINCREMENT,
    node_hash blob not NULL,
    parent_node_hash blob not NULL,
    height integer not NULL,
    shard blob not NULL,
    payload_id blob not NULL,
    payload_height integer not NULL,
    local_pledges blob not NULL,
    epoch integer not NULL,
    proposed_by blob not NULL,
    justify blob not NULL,
}

-- fetching by tree_node_hash will be a very common operation
create index nodes_index for nodes (tree_node_hash)


create table last_executed_height {
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    node_height integer not NULL
}

-- fetching by shard_id will be a very common operation
create index last_executed_height_index for last_executed_height (shard_id)


create table payload_votes {
    id integer not null primary key AUTOINCREMENT,
    payload_id blob not NULL,
    shard_id blob not NULL,
    node_height integer not NULL,
    hotstuff_tree_node blob not NULL
}

-- fetching by (payload_id, shard_id, node_height) will be a very common operation
create index payload_votes_index for payload_votes (payload_id, shard_id, node_height)


create table objects {
    id integer not null primary key AUTOINCREMENT
    shard_id blob not NULL,
    object_id blob not NULL,
    substate_state blob not NULL,
    object_pledge blob
}

-- fetching by (shard_id, object_id) will be a very common operation
create index objects_index for objects (shard_id, object_id)
