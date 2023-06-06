// @generated automatically by Diesel CLI.

diesel::table! {
    blocks (id) {
        id -> Integer,
        block_id -> Text,
        parent_block_id -> Text,
        height -> BigInt,
        leader_round -> BigInt,
        epoch -> BigInt,
        proposed_by -> Text,
        justify -> Text,
        prepared -> Text,
        precommitted -> Text,
        committed -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    committed_transaction_pool (id) {
        id -> Integer,
        transaction_id -> Text,
        decision -> Text,
        fee -> BigInt,
        is_ready -> Bool,
        created_at -> Timestamp,
    }
}

diesel::table! {
    high_qcs (id) {
        id -> Integer,
        epoch -> BigInt,
        block_id -> Text,
        height -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    leaf_blocks (id) {
        id -> Integer,
        epoch -> BigInt,
        block_id -> Text,
        block_height -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    new_transaction_pool (id) {
        id -> Integer,
        transaction_id -> Text,
        decision -> Text,
        fee -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    precommitted_transaction_pool (id) {
        id -> Integer,
        transaction_id -> Text,
        decision -> Text,
        fee -> BigInt,
        is_ready -> Bool,
        created_at -> Timestamp,
    }
}

diesel::table! {
    prepared_transaction_pool (id) {
        id -> Integer,
        transaction_id -> Text,
        decision -> Text,
        fee -> BigInt,
        is_ready -> Bool,
        created_at -> Timestamp,
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
        fee_paid_for_created_justify -> BigInt,
        fee_paid_for_deleted_justify -> BigInt,
        created_at_epoch -> Nullable<BigInt>,
        destroyed_at_epoch -> Nullable<BigInt>,
        created_justify_leader -> Nullable<Text>,
        destroyed_justify_leader -> Nullable<Text>,
        created_timestamp -> Timestamp,
        destroyed_timestamp -> Nullable<Timestamp>,
    }
}

diesel::table! {
    transactions (id) {
        id -> Integer,
        transaction_id -> Text,
        fee_instructions -> Text,
        instructions -> Text,
        sender_public_key -> Text,
        signature -> Text,
        meta -> Text,
        result -> Text,
        involved_shards -> Text,
        is_finalized -> Bool,
        created_at -> Timestamp,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    blocks,
    committed_transaction_pool,
    high_qcs,
    leaf_blocks,
    new_transaction_pool,
    precommitted_transaction_pool,
    prepared_transaction_pool,
    substates,
    transactions,
);
