table! {
    payloads (id) {
        id -> Integer,
        payload_id -> Binary,
        instructions -> Binary,
        public_nonce -> Binary,
        scalar -> Binary,
        fee -> Integer,
        sender_public_key -> Binary,
        meta -> Binary,
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
        hotstuff_tree_node -> Binary,
    }
}

table! {
    last_executed_heights (id) {
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
        node_height -> Integer,
        hotstuff_tree_node -> Binary,
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
    payloads,
    votes,
    leaf_nodes,
    last_voted_heights,
    lock_node_and_heights,
    nodes,
    last_executed_heights,
    payload_votes,
    objects
);
