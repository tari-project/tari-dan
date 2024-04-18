// @generated automatically by Diesel CLI.

diesel::table! {
    bmt_cache (epoch) {
        epoch -> BigInt,
        bmt -> Binary,
    }
}

diesel::table! {
    epochs (epoch) {
        epoch -> BigInt,
        validator_node_mr -> Binary,
    }
}

diesel::table! {
    base_layer_block_info (hash) {
        hash -> Binary,
        height -> BigInt,
    }
}

diesel::table! {
    metadata (key_name) {
        key_name -> Binary,
        value -> Binary,
    }
}

diesel::table! {
    templates (id) {
        id -> Integer,
        template_name -> Text,
        expected_hash -> Binary,
        template_address -> Binary,
        url -> Text,
        height -> BigInt,
        template_type -> Text,
        compiled_code -> Nullable<Binary>,
        flow_json -> Nullable<Text>,
        status -> Text,
        wasm_path -> Nullable<Text>,
        manifest -> Nullable<Text>,
        added_at -> Timestamp,
    }
}

diesel::table! {
    validator_nodes (id) {
        id -> Integer,
        public_key -> Binary,
        shard_key -> Binary,
        epoch -> BigInt,
        fee_claim_public_key -> Binary,
        address -> Text,
        sidechain_id -> Nullable<Binary>,
    }
}

diesel::table! {
    committees (id) {
        id -> Integer,
        public_key -> Binary,
        epoch -> BigInt,
        committee_bucket -> BigInt,
    }
}

diesel::allow_tables_to_appear_in_same_query!(bmt_cache, committees, epochs, metadata, templates, validator_nodes);
