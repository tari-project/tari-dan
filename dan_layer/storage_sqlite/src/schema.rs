table! {
    high_qcs (id) {
        id -> Integer,
        shard_id -> Binary,
        height -> BigInt,
        qc_json -> Text,
    }
}

table! {
    instructions (id) {
        id -> Integer,
        hash -> Binary,
        node_id -> Integer,
        template_id -> Integer,
        method -> Text,
        args -> Binary,
        sender -> Binary,
    }
}

table! {
    last_executed_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> BigInt,
    }
}

table! {
    last_voted_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> BigInt,
    }
}

table! {
    leaf_nodes (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
    }
}

table! {
    lock_node_and_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
    }
}

table! {
    locked_qc (id) {
        id -> Integer,
        message_type -> Integer,
        view_number -> BigInt,
        node_hash -> Binary,
        signature -> Nullable<Binary>,
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
    }
}

table! {
    objects (id) {
        id -> Integer,
        shard_id -> Binary,
        payload_id -> Binary,
        object_id -> Binary,
        node_height -> BigInt,
        current_state -> Text,
        object_pledge -> Text,
    }
}

table! {
    payload_votes (id) {
        id -> Integer,
        payload_id -> Binary,
        shard_id -> Binary,
        node_height -> BigInt,
        hotstuff_tree_node -> Text,
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
    }
}

table! {
    prepare_qc (id) {
        id -> Integer,
        message_type -> Integer,
        view_number -> BigInt,
        node_hash -> Binary,
        signature -> Nullable<Binary>,
    }
}

table! {
    state_keys (schema_name, key_name) {
        schema_name -> Text,
        key_name -> Binary,
        value -> Binary,
    }
}

table! {
    state_op_log (id) {
        id -> Integer,
        height -> BigInt,
        merkle_root -> Nullable<Binary>,
        operation -> Text,
        schema -> Text,
        key -> Binary,
        value -> Nullable<Binary>,
    }
}

table! {
    state_tree (id) {
        id -> Integer,
        version -> Integer,
        is_current -> Bool,
        data -> Binary,
    }
}

table! {
    substate_changes (id) {
        id -> Integer,
        shard_id -> Binary,
        substate_change -> Text,
        tree_node_hash -> Binary,
    }
}

table! {
    votes (id) {
        id -> Integer,
        tree_node_hash -> Binary,
        shard_id -> Binary,
        address -> Binary,
        node_height -> BigInt,
        vote_message -> Text,
    }
}

joinable!(instructions -> nodes (node_id));

allow_tables_to_appear_in_same_query!(
    high_qcs,
    instructions,
    last_executed_heights,
    last_voted_heights,
    leaf_nodes,
    lock_node_and_heights,
    locked_qc,
    metadata,
    nodes,
    objects,
    payload_votes,
    payloads,
    prepare_qc,
    state_keys,
    state_op_log,
    state_tree,
    substate_changes,
    votes,
);
