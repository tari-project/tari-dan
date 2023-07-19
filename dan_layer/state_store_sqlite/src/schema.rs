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
        qc_id -> Text,
        commands -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    block_missing_txs(id) {
        id -> Integer,
        block_id -> Text,
        transaction_ids -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    missing_tx(id) {
        id -> Integer,
        transaction_id -> Text,
        block_id -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    high_qcs (id) {
        id -> Integer,
        epoch -> BigInt,
        block_id -> Text,
        qc_id -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    last_executed (id) {
        id -> Integer,
        epoch -> BigInt,
        block_id -> Text,
        height -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    last_proposed (id) {
        id -> Integer,
        epoch -> BigInt,
        block_id -> Text,
        height -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    last_voted (id) {
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
    locked_block (id) {
        id -> Integer,
        epoch -> BigInt,
        block_id -> Text,
        height -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    quorum_certificates (id) {
        id -> Integer,
        qc_id -> Text,
        json -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    substates (id) {
        id -> Integer,
        shard_id -> Text,
        address -> Text,
        version -> Integer,
        data -> Text,
        state_hash -> Text,
        created_by_transaction -> Text,
        created_justify -> Text,
        created_block -> Text,
        created_height -> BigInt,
        destroyed_by_transaction -> Nullable<Text>,
        destroyed_justify -> Nullable<Text>,
        destroyed_by_block -> Nullable<Text>,
        created_at_epoch -> BigInt,
        destroyed_at_epoch -> Nullable<BigInt>,
        read_locks -> Integer,
        is_locked_w -> Bool,
        locked_by -> Nullable<Text>,
        created_at -> Timestamp,
        destroyed_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    transaction_pool (id) {
        id -> Integer,
        transaction_id -> Text,
        involved_shards -> Text,
        original_decision -> Text,
        changed_decision -> Nullable<Text>,
        evidence -> Text,
        fee -> BigInt,
        stage -> Text,
        is_ready -> Bool,
        updated_at -> Timestamp,
        created_at -> Timestamp,
    }
}

diesel::table! {
    transactions (id) {
        id -> Integer,
        transaction_id -> Text,
        fee_instructions -> Text,
        instructions -> Text,
        signature -> Text,
        inputs -> Text,
        input_refs -> Text,
        outputs -> Text,
        filled_inputs -> Text,
        filled_outputs -> Text,
        result -> Text,
        final_decision -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    votes (id) {
        id -> Integer,
        hash -> Text,
        epoch -> BigInt,
        block_id -> Text,
        decision -> Integer,
        sender_leaf_hash -> Text,
        signature -> Text,
        merkle_proof -> Text,
        created_at -> Timestamp,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    blocks,
    high_qcs,
    last_executed,
    last_proposed,
    last_voted,
    leaf_blocks,
    locked_block,
    quorum_certificates,
    substates,
    transaction_pool,
    transactions,
    votes,
);
