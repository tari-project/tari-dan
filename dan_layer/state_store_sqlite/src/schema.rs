// @generated automatically by Diesel CLI.

diesel::table! {
    blocks (id) {
        id -> Integer,
        block_id -> Text,
        parent_block_id -> Text,
        merkle_root -> Text,
        network -> Text,
        height -> BigInt,
        epoch -> BigInt,
        proposed_by -> Text,
        qc_id -> Text,
        command_count -> BigInt,
        commands -> Text,
        total_leader_fee -> BigInt,
        is_committed -> Bool,
        is_processed -> Bool,
        is_dummy -> Bool,
        foreign_indexes -> Text,
        signature -> Nullable<Text>,
        created_at -> Timestamp,
        block_time -> Nullable<BigInt>,
        timestamp -> BigInt,
        base_layer_block_hash -> Text,
    }
}

diesel::table! {
    foreign_proposals (id) {
        id -> Integer,
        bucket -> Integer,
        block_id -> Text,
        state -> Text,
        proposed_height -> Nullable<BigInt>,
        transactions -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    foreign_receive_counters (id) {
        id -> Integer,
        counters -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    foreign_send_counters (id) {
        id -> Integer,
        block_id -> Text,
        counters -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    high_qcs (id) {
        id -> Integer,
        block_id -> Text,
        block_height -> BigInt,
        qc_id -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    last_executed (id) {
        id -> Integer,
        block_id -> Text,
        height -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    last_proposed (id) {
        id -> Integer,
        block_id -> Text,
        height -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    last_sent_vote (id) {
        id -> Integer,
        epoch -> BigInt,
        block_id -> Text,
        block_height -> BigInt,
        decision -> Integer,
        signature -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    last_voted (id) {
        id -> Integer,
        block_id -> Text,
        height -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    leaf_blocks (id) {
        id -> Integer,
        block_id -> Text,
        block_height -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    locked_block (id) {
        id -> Integer,
        block_id -> Text,
        height -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    locked_outputs (id) {
        id -> Integer,
        block_id -> Text,
        transaction_id -> Text,
        substate_address -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    missing_transactions (id) {
        id -> Integer,
        block_id -> Text,
        block_height -> BigInt,
        transaction_id -> Text,
        is_awaiting_execution -> Bool,
        created_at -> Timestamp,
    }
}

diesel::table! {
    parked_blocks (id) {
        id -> Integer,
        block_id -> Text,
        parent_block_id -> Text,
        merkle_root -> Text,
        network -> Text,
        height -> BigInt,
        epoch -> BigInt,
        proposed_by -> Text,
        justify -> Text,
        command_count -> BigInt,
        commands -> Text,
        total_leader_fee -> BigInt,
        foreign_indexes -> Text,
        signature -> Nullable<Text>,
        created_at -> Timestamp,
        block_time -> Nullable<BigInt>,
        timestamp -> BigInt,
        base_layer_block_hash -> Text,
    }
}

diesel::table! {
    pending_state_tree_diffs (id) {
        id -> Integer,
        block_id -> Text,
        block_height -> BigInt,
        diff_json -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    quorum_certificates (id) {
        id -> Integer,
        qc_id -> Text,
        block_id -> Text,
        json -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    state_tree (id) {
        id -> Integer,
        key -> Text,
        node -> Text,
        is_stale -> Bool,
    }
}

diesel::table! {
    substates (id) {
        id -> Integer,
        address -> Text,
        substate_id -> Text,
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
        original_decision -> Text,
        local_decision -> Nullable<Text>,
        remote_decision -> Nullable<Text>,
        evidence -> Text,
        remote_evidence -> Nullable<Text>,
        transaction_fee -> BigInt,
        leader_fee -> Nullable<BigInt>,
        global_exhaust_burn -> Nullable<BigInt>,
        stage -> Text,
        pending_stage -> Nullable<Text>,
        is_ready -> Bool,
        updated_at -> Timestamp,
        created_at -> Timestamp,
    }
}

diesel::table! {
    transaction_pool_history (history_id) {
        history_id -> Nullable<Integer>,
        id -> Integer,
        transaction_id -> Text,
        original_decision -> Text,
        local_decision -> Nullable<Text>,
        remote_decision -> Nullable<Text>,
        evidence -> Text,
        transaction_fee -> BigInt,
        leader_fee -> Nullable<BigInt>,
        global_exhaust_burn -> Nullable<BigInt>,
        stage -> Text,
        new_stage -> Text,
        is_ready -> Bool,
        new_is_ready -> Bool,
        updated_at -> Timestamp,
        created_at -> Timestamp,
        change_time -> Nullable<Timestamp>,
    }
}

diesel::table! {
    transaction_pool_state_updates (id) {
        id -> Integer,
        block_id -> Text,
        block_height -> BigInt,
        transaction_id -> Text,
        stage -> Text,
        evidence -> Text,
        is_ready -> Bool,
        local_decision -> Text,
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
        filled_inputs -> Text,
        resulting_outputs -> Nullable<Text>,
        result -> Nullable<Text>,
        execution_time_ms -> Nullable<BigInt>,
        final_decision -> Nullable<Text>,
        finalized_at -> Nullable<Timestamp>,
        abort_details -> Nullable<Text>,
        min_epoch -> Nullable<BigInt>,
        max_epoch -> Nullable<BigInt>,
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
        created_at -> Timestamp,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    blocks,
    foreign_proposals,
    foreign_receive_counters,
    foreign_send_counters,
    high_qcs,
    last_executed,
    last_proposed,
    last_sent_vote,
    last_voted,
    leaf_blocks,
    locked_block,
    locked_outputs,
    missing_transactions,
    parked_blocks,
    pending_state_tree_diffs,
    quorum_certificates,
    state_tree,
    substates,
    transaction_pool,
    transaction_pool_history,
    transaction_pool_state_updates,
    transactions,
    votes,
);
