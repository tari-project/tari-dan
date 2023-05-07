// @generated automatically by Diesel CLI.

diesel::table! {
    current_leader_states (id) {
        id -> Integer,
        payload_id -> Binary,
        shard_id -> Binary,
        leader_round -> BigInt,
        leader -> Binary,
        timestamp -> Timestamp,
    }
}

diesel::table! {
    high_qcs (id) {
        id -> Integer,
        shard_id -> Binary,
        payload_id -> Binary,
        height -> BigInt,
        qc_json -> Text,
        identity -> Binary,
        timestamp -> Timestamp,
    }
}

diesel::table! {
    last_executed_heights (id) {
        id -> Integer,
        payload_id -> Binary,
        shard_id -> Binary,
        node_height -> BigInt,
        timestamp -> Timestamp,
    }
}

diesel::table! {
    last_voted_heights (id) {
        id -> Integer,
        payload_id -> Binary,
        shard_id -> Binary,
        node_height -> BigInt,
        leader_round -> BigInt,
        timestamp -> Timestamp,
    }
}

diesel::table! {
    leader_proposals (id) {
        id -> Integer,
        payload_id -> Binary,
        shard_id -> Binary,
        payload_height -> BigInt,
        leader_round -> BigInt,
        node_hash -> Binary,
        hotstuff_tree_node -> Text,
        timestamp -> Timestamp,
    }
}

diesel::table! {
    leaf_nodes (id) {
        id -> Integer,
        shard_id -> Binary,
        payload_id -> Binary,
        payload_height -> BigInt,
        tree_node_hash -> Binary,
        node_height -> BigInt,
        timestamp -> Timestamp,
    }
}

diesel::table! {
    lock_node_and_heights (id) {
        id -> Integer,
        payload_id -> Binary,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
        timestamp -> Timestamp,
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
        leader_round -> BigInt,
        local_pledges -> Text,
        epoch -> BigInt,
        proposed_by -> Binary,
        justify -> Text,
        timestamp -> Timestamp,
    }
}

diesel::table! {
    payloads (id) {
        id -> Integer,
        payload_id -> Binary,
        fee_instructions -> Text,
        instructions -> Text,
        public_nonce -> Binary,
        scalar -> Binary,
        sender_address -> Binary,
        meta -> Text,
        result -> Nullable<Text>,
        timestamp -> Timestamp,
        is_finalized -> Bool,
    }
}

diesel::table! {
    received_votes (id) {
        id -> Integer,
        tree_node_hash -> Binary,
        address -> Binary,
        vote_message -> Text,
        timestamp -> Timestamp,
    }
}

diesel::table! {
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

diesel::table! {
    substates (id) {
        id -> Integer,
        shard_id -> Binary,
        address -> Text,
        version -> BigInt,
        data -> Text,
        created_by_payload_id -> Binary,
        created_justify -> Nullable<Text>,
        created_node_hash -> Binary,
        created_height -> BigInt,
        destroyed_by_payload_id -> Nullable<Binary>,
        destroyed_justify -> Nullable<Text>,
        destroyed_node_hash -> Nullable<Binary>,
        destroyed_height -> Nullable<BigInt>,
        created_timestamp -> Timestamp,
        destroyed_timestamp -> Nullable<Timestamp>,
        fee_paid_for_created_justify -> BigInt,
        fee_paid_for_deleted_justify -> BigInt,
        created_at_epoch -> Nullable<BigInt>,
        destroyed_at_epoch -> Nullable<BigInt>,
        created_justify_leader -> Nullable<Text>,
        destroyed_justify_leader -> Nullable<Text>,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    current_leader_states,
    high_qcs,
    last_executed_heights,
    last_voted_heights,
    leader_proposals,
    leaf_nodes,
    lock_node_and_heights,
    nodes,
    payloads,
    received_votes,
    shard_pledges,
    substates,
);
