table! {
    high_qcs (id) {
        id -> Integer,
        shard_id -> Binary,
        height -> BigInt,
        qc_json -> Text,
    }
}

table! {
    last_executed_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> BigInt,
    }
}

table! {
    last_voted_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> BigInt,
    }
}

table! {
    leader_proposals (id) {
        id -> Integer,
        payload_id -> Binary,
        shard_id -> Binary,
        payload_height -> BigInt,
        node_hash -> Binary,
        hotstuff_tree_node -> Text,
    }
}

table! {
    leaf_nodes (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
    }
}

table! {
    lock_node_and_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
    }
}

table! {
    metadata (key) {
        key -> Binary,
        value -> Binary,
    }
}

table! {
    nodes (id) {
        id -> Integer,
        node_hash -> Binary,
        parent_node_hash -> Binary,
        height -> BigInt,
        shard -> Binary,
        payload_id -> Binary,
        payload_height -> BigInt,
        local_pledges -> Text,
        epoch -> BigInt,
        proposed_by -> Binary,
        justify -> Text,
    }
}

table! {
    objects (id) {
        id -> Integer,
        shard_id -> Binary,
        payload_id -> Binary,
        object_id -> Binary,
        node_height -> BigInt,
        current_state -> Text,
        object_pledge -> Text,
    }
}

table! {
    payloads (id) {
        id -> Integer,
        payload_id -> Binary,
        instructions -> Text,
        public_nonce -> Binary,
        scalar -> Binary,
        fee -> BigInt,
        sender_public_key -> Binary,
        meta -> Text,
    }
}

table! {
    received_votes (id) {
        id -> Integer,
        tree_node_hash -> Binary,
        shard_id -> Binary,
        address -> Binary,
        vote_message -> Text,
    }
}

table! {
    substate_changes (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        substate_change -> Text,
    }
}

allow_tables_to_appear_in_same_query!(
    high_qcs,
    last_executed_heights,
    last_voted_heights,
    leader_proposals,
    leaf_nodes,
    lock_node_and_heights,
    metadata,
    nodes,
    objects,
    payloads,
    received_votes,
    substate_changes,
);
