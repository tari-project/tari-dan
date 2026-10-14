// @generated automatically by Diesel CLI.

diesel::table! {
    block_headers (id) {
        id -> Integer,
        epoch -> BigInt,
        height -> BigInt,
        block_hash -> Binary,
        block_output_merkle_root -> Binary,
        validator_node_merkle_root -> Binary,
        created_at -> Timestamp,
    }
}

diesel::table! {
    bmt_cache (epoch) {
        epoch -> BigInt,
        bmt -> Binary,
    }
}

diesel::table! {
    committees (id) {
        id -> Integer,
        validator_node_id -> Integer,
        epoch -> BigInt,
        shard_start -> Integer,
        shard_end -> Integer,
    }
}

diesel::table! {
    epochs (epoch) {
        epoch -> BigInt,
        epoch_hash -> Binary,
        validator_set_hash -> Nullable<Binary>,
    }
}

diesel::table! {
    layer_one_transactions (id) {
        id -> Integer,
        epoch -> BigInt,
        payload_type -> Text,
        payload -> Text,
        submitted_at -> Nullable<Timestamp>,
        is_observed -> Bool,
    }
}

diesel::table! {
    metadata (key_name) {
        key_name -> Binary,
        value -> Binary,
    }
}

diesel::table! {
    validator_set_leaves (epoch, shard_start, shard_end) {
        epoch -> BigInt,
        shard_start -> Integer,
        shard_end -> Integer,
        leaf_hash -> Binary,
    }
}

diesel::table! {
    validator_nodes (id) {
        id -> Integer,
        public_key -> Binary,
        address -> Text,
        shard_key -> Binary,
        start_epoch -> BigInt,
        end_epoch -> Nullable<BigInt>,
        fee_claim_public_key -> Binary,
        power -> BigInt,
    }
}

diesel::joinable!(committees -> validator_nodes (validator_node_id));
diesel::joinable!(validator_set_leaves -> epochs (epoch));

diesel::allow_tables_to_appear_in_same_query!(
    block_headers,
    bmt_cache,
    committees,
    epochs,
    layer_one_transactions,
    metadata,
    validator_nodes,
    validator_set_leaves,
);
