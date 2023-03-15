// @generated automatically by Diesel CLI.

diesel::table! {
    accounts (id) {
        id -> Integer,
        name -> Text,
        address -> Text,
        owner_key_index -> BigInt,
        balance -> BigInt,
        created_at -> Timestamp,
        updated_at -> Timestamp,
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
    outputs (id) {
        id -> Integer,
        account_id -> Integer,
        vault_id -> Integer,
        commitment -> Text,
        value -> BigInt,
        sender_public_nonce -> Nullable<Text>,
        secret_key_index -> BigInt,
        public_asset_tag -> Nullable<Text>,
        status -> Text,
        locked_at -> Nullable<Timestamp>,
        locked_by_proof -> Nullable<Integer>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
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
        sender_address -> Text,
        fee -> BigInt,
        meta -> Text,
        result -> Nullable<Text>,
        qcs -> Nullable<Text>,
        status -> Text,
        dry_run -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    vaults (id) {
        id -> Integer,
        account_id -> Integer,
        address -> Text,
        resource_address -> Text,
        balance -> BigInt,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(outputs -> accounts (account_id));
diesel::joinable!(outputs -> vaults (vault_id));
diesel::joinable!(proofs -> accounts (account_id));
diesel::joinable!(proofs -> vaults (vault_id));
diesel::joinable!(vaults -> accounts (account_id));

diesel::allow_tables_to_appear_in_same_query!(
    accounts,
    config,
    key_manager_states,
    outputs,
    proofs,
    substates,
    transactions,
    vaults,
);
