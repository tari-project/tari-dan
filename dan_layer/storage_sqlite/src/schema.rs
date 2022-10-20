// @generated automatically by Diesel CLI.

diesel::table! {
    high_qcs (id) {
        id -> Integer,
        shard_id -> Binary,
        height -> BigInt,
        qc_json -> Text,
    }
}

diesel::table! {
    last_executed_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> BigInt,
    }
}

diesel::table! {
    last_voted_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> BigInt,
    }
}

diesel::table! {
    leader_proposals (id) {
        id -> Integer,
        payload_id -> Binary,
        shard_id -> Binary,
        payload_height -> BigInt,
        node_hash -> Binary,
        hotstuff_tree_node -> Text,
    }
}

diesel::table! {
    leaf_nodes (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
    }
}

diesel::table! {
    lock_node_and_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
    }
}

diesel::table! {
    metadata (key) {
        key -> Binary,
        value -> Binary,
    }
}

diesel::table! {
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

diesel::table! {
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

diesel::table! {
    received_votes (id) {
        id -> Integer,
        tree_node_hash -> Binary,
        shard_id -> Binary,
        address -> Binary,
        vote_message -> Text,
    }
}

diesel::table! {
    substates (id) {
        id -> Integer,
        substate_type -> Text,
        shard_id -> Binary,
        node_height -> BigInt,
        data -> Nullable<Binary>,
        created_by_payload_id -> Binary,
        deleted_by_payload_id -> Nullable<Binary>,
        justify -> Nullable<Text>,
        is_draft -> Bool,
        tree_node_hash -> Nullable<Binary>,
        pledged_to_payload_id -> Nullable<Binary>,
        pledged_until_height -> Nullable<BigInt>,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
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
    substates,
);
