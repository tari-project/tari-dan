table! {
    high_qcs (id) {
        id -> Integer,
        shard_id -> Binary,
        height -> Integer,
        is_highest -> Integer,
        qc_json -> Text,
    }
}

table! {
    metadata (key) {
        key -> Binary,
        value -> Binary,
    }
}

table! {
    templates (id) {
        id -> Integer,
        template_address -> Binary,
        url -> Text,
        height -> Integer,
        compiled_code -> Binary,
    }
}

allow_tables_to_appear_in_same_query!(
    high_qcs,
    metadata,
    templates,
);
