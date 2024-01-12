// @generated automatically by Diesel CLI.

diesel::table! {
    accounts (id) {
        id -> Integer,
        name -> Text,
        address -> Text,
        owner_key_index -> BigInt,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        is_default -> Bool,
    }
}

diesel::table! {
    auth_status (id) {
        id -> Integer,
        user_decided -> Bool,
        granted -> Bool,
        token -> Nullable<Text>,
        revoked -> Bool,
    }
}

diesel::table! {
    config (id) {
        id -> Integer,
        key -> Text,
        value -> Text,
        is_encrypted -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    key_manager_states (id) {
        id -> Integer,
        branch_seed -> Text,
        index -> BigInt,
        is_active -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    non_fungible_tokens (id) {
        id -> Integer,
        vault_id -> Integer,
        nft_id -> Text,
        metadata -> Text,
        is_burned -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    outputs (id) {
        id -> Integer,
        account_id -> Integer,
        vault_id -> Integer,
        commitment -> Text,
        value -> BigInt,
        sender_public_nonce -> Nullable<Text>,
        encryption_secret_key_index -> BigInt,
        public_asset_tag -> Nullable<Text>,
        status -> Text,
        locked_at -> Nullable<Timestamp>,
        locked_by_proof -> Nullable<Integer>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        encrypted_data -> Binary,
    }
}

diesel::table! {
    proofs (id) {
        id -> Integer,
        account_id -> Integer,
        vault_id -> Integer,
        transaction_hash -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    substates (id) {
        id -> Integer,
        module_name -> Nullable<Text>,
        address -> Text,
        parent_address -> Nullable<Text>,
        version -> Integer,
        transaction_hash -> Text,
        template_address -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    transactions (id) {
        id -> Integer,
        hash -> Text,
        instructions -> Text,
        signature -> Text,
        sender_public_key -> Text,
        fee_instructions -> Text,
        meta -> Text,
        result -> Nullable<Text>,
        qcs -> Nullable<Text>,
        final_fee -> Nullable<BigInt>,
        status -> Text,
        dry_run -> Bool,
        min_epoch -> Nullable<BigInt>,
        max_epoch -> Nullable<BigInt>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        json_result -> Nullable<Text>,
        executed_time_ms -> Nullable<BigInt>,
        finalized_time_ms -> Nullable<BigInt>,
    }
}

diesel::table! {
    vaults (id) {
        id -> Integer,
        account_id -> Integer,
        address -> Text,
        resource_address -> Text,
        resource_type -> Text,
        balance -> BigInt,
        token_symbol -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(non_fungible_tokens -> vaults (vault_id));
diesel::joinable!(outputs -> accounts (account_id));
diesel::joinable!(outputs -> vaults (vault_id));
diesel::joinable!(proofs -> accounts (account_id));
diesel::joinable!(proofs -> vaults (vault_id));
diesel::joinable!(vaults -> accounts (account_id));

diesel::allow_tables_to_appear_in_same_query!(
    accounts,
    auth_status,
    config,
    key_manager_states,
    non_fungible_tokens,
    outputs,
    proofs,
    substates,
    transactions,
    vaults,
);
