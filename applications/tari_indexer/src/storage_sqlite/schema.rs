// @generated automatically by Diesel CLI.

diesel::table! {
    template_catalogue (id) {
        id -> Integer,
        template_address -> Text,
        template_name -> Text,
        author_public_key -> Text,
        binary_hash -> Text,
        at_epoch -> BigInt,
        metadata_hash -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    epoch_checkpoints (id) {
        id -> Integer,
        epoch -> BigInt,
        shard_group -> Text,
        json_data -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    events (id) {
        id -> BigInt,
        template_address -> Text,
        tx_hash -> Text,
        topic -> Text,
        payload -> Text,
        substate_id -> Nullable<Text>,
        resource_address -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    key_values (id) {
        id -> Integer,
        key -> Text,
        value -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    substate_transitions (id) {
        id -> Integer,
        shard -> Integer,
        state_version -> BigInt,
        epoch -> BigInt,
        substate_id -> Text,
        version -> Integer,
        substate_type -> Text,
        is_up -> Bool,
        value_hash -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    substates (id) {
        id -> Integer,
        address -> Text,
        version -> Integer,
        data -> Text,
        template_address -> Nullable<Text>,
        module_name -> Nullable<Text>,
        updated_at -> Timestamp,
        created_at -> Timestamp,
    }
}

diesel::table! {
    transaction_receipts (id) {
        id -> Integer,
        address -> Text,
        data -> Text,
        created_at -> Timestamp,
        outcome -> Text,
        total_fees_paid -> BigInt,
    }
}

diesel::table! {
    transactions (id) {
        id -> Integer,
        transaction_id -> Text,
        body -> Text,
        created_at -> Timestamp,
        rejected_reason -> Nullable<Text>,
        rejected_at -> Nullable<Timestamp>,
        retention_epoch -> BigInt,
        source -> Text,
    }
}

diesel::table! {
    watched_substates (id) {
        id -> Integer,
        component_address -> Text,
        template_address -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    utxos (id) {
        id -> Integer,
        commitment -> Text,
        public_nonce -> Text,
        version -> Integer,
        resource_address -> Text,
        shard -> Integer,
        state_version -> BigInt,
        output -> Nullable<Binary>,
        utxo_tag -> Integer,
        epoch -> BigInt,
        is_spent -> Bool,
        is_burnt -> Bool,
        is_frozen -> Bool,
        created_at -> Timestamp,
    }
}

diesel::table! {
    verified_state_roots (id) {
        id -> Integer,
        epoch -> BigInt,
        shard_group -> Text,
        block_height -> BigInt,
        block_hash -> Text,
        state_merkle_root -> Text,
        validated_at -> Timestamp,
    }
}

diesel::table! {
    substate_cache (substate_id) {
        substate_id -> Text,
        version -> Nullable<Integer>,
        verified -> Bool,
        substate_result -> Binary,
        cached_at -> BigInt,
    }
}

diesel::table! {
    substate_cache_invalidations (substate_id) {
        substate_id -> Text,
        state_version -> BigInt,
        substate_version -> Integer,
        invalidated_at -> BigInt,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    epoch_checkpoints,
    events,
    key_values,
    substate_cache,
    substate_cache_invalidations,
    substate_transitions,
    substates,
    template_catalogue,
    transaction_receipts,
    transactions,
    utxos,
    verified_state_roots,
    watched_substates,
);
