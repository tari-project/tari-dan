table! {
    payload_table (id) {
        id -> Integer,
        payload_id -> Binary,
        payload -> Binary,
    }
}

table! {
    votes (id) {
        id -> Integer,
        tree_node_hash -> Binary,
        shard_id -> Binary,
        address -> Binary,
        node_height -> Integer,
    }
}

table! {
    leaf_nodes (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> Integer,
    }
}

table! {
    last_voted_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> Integer,
    }
}

table! {
    lock_node_and_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> Integer,
    }
}

table! {
    nodes (id) {
        id -> Integer,
        tree_node_hash -> Binary,
        hot_stuff_tree_node -> Binary,
    }
}

table! {
    last_executed_height (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> Integer,
    }
}

table! {
    payload_votes (id) {
        id -> Integer,
        payload_id -> Binary,
        shard_id -> Binary,
        node_height -> Binary,
        hot_stuff_tree_node -> Binary,
    }
}

table! {
    objects (id) {
        id -> Integer,
        shard_id -> Binary,
        object_id -> Binary,
        substate_state -> Binary,
        object_pledge -> Binary,
    }
}

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
