// @generated automatically by Diesel CLI.

diesel::table! {
    block_diffs (id) {
        id -> Integer,
        block_id -> Text,
        transaction_id -> Text,
        substate_id -> Text,
        version -> Integer,
        shard -> Integer,
        change -> Text,
        state -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    blocks (id) {
        id -> Integer,
        block_id -> Text,
        parent_block_id -> Text,
        merkle_root -> Text,
        network -> Text,
        height -> BigInt,
        epoch -> BigInt,
        shard_group -> Integer,
        proposed_by -> Text,
        qc_id -> Text,
        command_count -> BigInt,
        commands -> Text,
        total_leader_fee -> BigInt,
        is_committed -> Bool,
        is_justified -> Bool,
        is_dummy -> Bool,
        foreign_indexes -> Text,
        signature -> Nullable<Text>,
        block_time -> Nullable<BigInt>,
        timestamp -> BigInt,
        base_layer_block_height -> BigInt,
        base_layer_block_hash -> Text,
        extra_data -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    burnt_utxos (id) {
        id -> Integer,
        substate_id -> Text,
        substate -> Text,
        base_layer_block_height -> BigInt,
        proposed_in_block -> Nullable<Text>,
        proposed_in_block_height -> Nullable<BigInt>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    diagnostic_deleted_blocks (id) {
        id -> Integer,
        block_id -> Text,
        parent_block_id -> Text,
        merkle_root -> Text,
        network -> Text,
        height -> BigInt,
        epoch -> BigInt,
        shard_group -> Integer,
        proposed_by -> Text,
        qc_id -> Text,
        command_count -> BigInt,
        commands -> Text,
        total_leader_fee -> BigInt,
        is_committed -> Bool,
        is_justified -> Bool,
        is_dummy -> Bool,
        foreign_indexes -> Text,
        signature -> Nullable<Text>,
        block_time -> Nullable<BigInt>,
        timestamp -> BigInt,
        base_layer_block_height -> BigInt,
        base_layer_block_hash -> Text,
        extra_data -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    diagnostics_no_votes (id) {
        id -> Integer,
        block_id -> Text,
        block_height -> BigInt,
        reason_code -> Text,
        reason_text -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    epoch_checkpoints (id) {
        id -> Integer,
        epoch -> BigInt,
        commit_block -> Text,
        qcs -> Text,
        shard_roots -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    foreign_missing_transactions (id) {
        id -> Integer,
        parked_block_id -> Integer,
        transaction_id -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    foreign_parked_blocks (id) {
        id -> Integer,
        block_id -> Text,
        block -> Text,
        block_pledges -> Text,
        justify_qc -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    foreign_proposals (id) {
        id -> Integer,
        block_id -> Text,
        parent_block_id -> Text,
        merkle_root -> Text,
        network -> Text,
        height -> BigInt,
        epoch -> BigInt,
        shard_group -> Integer,
        proposed_by -> Text,
        qc -> Text,
        command_count -> BigInt,
        commands -> Text,
        total_leader_fee -> BigInt,
        foreign_indexes -> Text,
        signature -> Nullable<Text>,
        timestamp -> BigInt,
        base_layer_block_height -> BigInt,
        base_layer_block_hash -> Text,
        justify_qc_id -> Text,
        block_pledge -> Text,
        proposed_in_block -> Nullable<Text>,
        proposed_in_block_height -> Nullable<BigInt>,
        status -> Text,
        extra_data -> Nullable<Text>,
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
    foreign_substate_pledges (id) {
        id -> Integer,
        transaction_id -> Text,
        address -> Text,
        substate_id -> Text,
        version -> Integer,
        substate_value -> Nullable<Text>,
        shard_group -> Integer,
        lock_type -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    high_qcs (id) {
        id -> Integer,
        block_id -> Text,
        block_height -> BigInt,
        epoch -> BigInt,
        qc_id -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    last_executed (id) {
        id -> Integer,
        block_id -> Text,
        height -> BigInt,
        epoch -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    last_proposed (id) {
        id -> Integer,
        block_id -> Text,
        height -> BigInt,
        epoch -> BigInt,
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
        epoch -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    leaf_blocks (id) {
        id -> Integer,
        block_id -> Text,
        block_height -> BigInt,
        epoch -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    lock_conflicts (id) {
        id -> Integer,
        block_id -> Text,
        transaction_id -> Text,
        depends_on_tx -> Text,
        lock_type -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    locked_block (id) {
        id -> Integer,
        block_id -> Text,
        height -> BigInt,
        epoch -> BigInt,
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
        shard_group -> Integer,
        proposed_by -> Text,
        justify -> Text,
        command_count -> BigInt,
        commands -> Text,
        total_leader_fee -> BigInt,
        foreign_indexes -> Text,
        signature -> Nullable<Text>,
        timestamp -> BigInt,
        base_layer_block_height -> BigInt,
        base_layer_block_hash -> Text,
        foreign_proposals -> Text,
        extra_data -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    pending_state_tree_diffs (id) {
        id -> Integer,
        block_id -> Text,
        block_height -> BigInt,
        shard -> Integer,
        version -> BigInt,
        diff_json -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    quorum_certificates (id) {
        id -> Integer,
        qc_id -> Text,
        block_id -> Text,
        shard_group -> Integer,
        json -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    state_transitions (id) {
        id -> Integer,
        epoch -> BigInt,
        shard -> Integer,
        seq -> BigInt,
        substate_address -> Text,
        substate_id -> Text,
        version -> Integer,
        transition -> Text,
        state_hash -> Nullable<Text>,
        state_version -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    state_tree (id) {
        id -> Integer,
        shard -> Integer,
        key -> Text,
        node -> Text,
    }
}

diesel::table! {
    state_tree_shard_versions (id) {
        id -> Integer,
        shard -> Integer,
        version -> BigInt,
        created_at -> Timestamp,
    }
}

diesel::table! {
    substate_locks (id) {
        id -> Integer,
        block_id -> Text,
        transaction_id -> Text,
        substate_id -> Text,
        version -> Integer,
        lock -> Text,
        is_local_only -> Bool,
        created_at -> Timestamp,
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
        created_at_epoch -> BigInt,
        created_by_shard -> Integer,
        destroyed_by_transaction -> Nullable<Text>,
        destroyed_justify -> Nullable<Text>,
        destroyed_by_block -> Nullable<BigInt>,
        destroyed_at_epoch -> Nullable<BigInt>,
        destroyed_by_shard -> Nullable<Integer>,
        created_at -> Timestamp,
        destroyed_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    transaction_executions (id) {
        id -> Integer,
        block_id -> Text,
        transaction_id -> Text,
        resolved_inputs -> Text,
        resulting_outputs -> Text,
        result -> Text,
        execution_time_ms -> BigInt,
        abort_reason -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    transaction_pool (id) {
        id -> Integer,
        transaction_id -> Text,
        original_decision -> Text,
        local_decision -> Nullable<Text>,
        remote_decision -> Nullable<Text>,
        evidence -> Nullable<Text>,
        transaction_fee -> BigInt,
        leader_fee -> Nullable<Text>,
        stage -> Text,
        pending_stage -> Nullable<Text>,
        is_ready -> Bool,
        confirm_stage -> Nullable<Text>,
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
        evidence -> Nullable<Text>,
        new_evidence -> Nullable<Text>,
        transaction_fee -> Nullable<BigInt>,
        leader_fee -> Nullable<BigInt>,
        global_exhaust_burn -> Nullable<BigInt>,
        stage -> Text,
        new_stage -> Text,
        pending_stage -> Nullable<Text>,
        new_pending_stage -> Nullable<Text>,
        is_ready -> Bool,
        new_is_ready -> Bool,
        confirm_stage -> Nullable<Text>,
        new_confirm_stage -> Nullable<Text>,
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
        transaction_fee -> BigInt,
        leader_fee -> Nullable<Text>,
        remote_decision -> Nullable<Text>,
        is_applied -> Bool,
        created_at -> Timestamp,
    }
}

diesel::table! {
    transactions (id) {
        id -> Integer,
        transaction_id -> Text,
        fee_instructions -> Text,
        instructions -> Text,
        signatures -> Text,
        inputs -> Text,
        filled_inputs -> Text,
        resolved_inputs -> Nullable<Text>,
        resulting_outputs -> Nullable<Text>,
        result -> Nullable<Text>,
        execution_time_ms -> Nullable<BigInt>,
        final_decision -> Nullable<Text>,
        finalized_at -> Nullable<Timestamp>,
        outcome -> Nullable<Text>,
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

diesel::joinable!(foreign_missing_transactions -> foreign_parked_blocks (parked_block_id));

diesel::allow_tables_to_appear_in_same_query!(
    block_diffs,
    blocks,
    burnt_utxos,
    diagnostic_deleted_blocks,
    diagnostics_no_votes,
    epoch_checkpoints,
    foreign_missing_transactions,
    foreign_parked_blocks,
    foreign_proposals,
    foreign_receive_counters,
    foreign_send_counters,
    foreign_substate_pledges,
    high_qcs,
    last_executed,
    last_proposed,
    last_sent_vote,
    last_voted,
    leaf_blocks,
    lock_conflicts,
    locked_block,
    missing_transactions,
    parked_blocks,
    pending_state_tree_diffs,
    quorum_certificates,
    state_transitions,
    state_tree,
    state_tree_shard_versions,
    substate_locks,
    substates,
    transaction_executions,
    transaction_pool,
    transaction_pool_history,
    transaction_pool_state_updates,
    transactions,
    votes,
);
