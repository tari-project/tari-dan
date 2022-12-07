table! {
    high_qcs (id) {
        id -> Integer,
        shard_id -> Binary,
        height -> BigInt,
        qc_json -> Text,
        identity -> Binary,
        timestamp -> Timestamp,
    }
}

table! {
    last_executed_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> BigInt,
        timestamp -> Timestamp,
    }
}

table! {
    last_voted_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> BigInt,
        timestamp -> Timestamp,
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
        timestamp -> Timestamp,
    }
}

table! {
    leaf_nodes (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
        timestamp -> Timestamp,
    }
}

table! {
    lock_node_and_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
        timestamp -> Timestamp,
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
        timestamp -> Timestamp,
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
        timestamp -> Timestamp,
        result -> Nullable<Text>,
    }
}

table! {
    received_votes (id) {
        id -> Integer,
        tree_node_hash -> Binary,
        shard_id -> Binary,
        address -> Binary,
        vote_message -> Text,
        timestamp -> Timestamp,
    }
}

table! {
    shard_pledges (id) {
        id -> Integer,
        shard_id -> Binary,
        created_height -> BigInt,
        pledged_to_payload_id -> Binary,
        is_active -> Bool,
        completed_by_tree_node_hash -> Nullable<Binary>,
        abandoned_by_tree_node_hash -> Nullable<Binary>,
        timestamp -> Timestamp,
        updated_timestamp -> Nullable<Timestamp>,
    }
}

table! {
    substates (id) {
        id -> Integer,
        shard_id -> Binary,
        version -> BigInt,
        data -> Text,
        created_by_payload_id -> Binary,
        created_justify -> Text,
        created_node_hash -> Binary,
        created_height -> BigInt,
        destroyed_by_payload_id -> Nullable<Binary>,
        destroyed_justify -> Nullable<Text>,
        destroyed_node_hash -> Nullable<Binary>,
        destroyed_height -> Nullable<BigInt>,
        created_timestamp -> Timestamp,
        destroyed_timestamp -> Nullable<Timestamp>,
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
    payloads,
    received_votes,
    shard_pledges,
    substates,
);
