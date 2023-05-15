table! {
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

table! {
    auth_status(id) {
        id -> Integer,
        user_decided -> Bool,
        granted -> Bool,
        token -> Text,
        revoked -> Bool,
    }
}

table! {
    config (id) {
        id -> Integer,
        key -> Text,
        value -> Text,
        is_encrypted -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    key_manager_states (id) {
        id -> Integer,
        branch_seed -> Text,
        index -> BigInt,
        is_active -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
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

table! {
    proofs (id) {
        id -> Integer,
        account_id -> Integer,
        vault_id -> Integer,
        transaction_hash -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

table! {
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

table! {
    transactions (id) {
        id -> Integer,
        hash -> Text,
        instructions -> Text,
        signature -> Text,
        sender_address -> Text,
        fee_instructions -> Text,
        meta -> Text,
        result -> Nullable<Text>,
        transaction_failure -> Nullable<Text>,
        qcs -> Nullable<Text>,
        final_fee -> Nullable<BigInt>,
        status -> Text,
        dry_run -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
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

joinable!(outputs -> accounts (account_id));
joinable!(outputs -> vaults (vault_id));
joinable!(proofs -> accounts (account_id));
joinable!(proofs -> vaults (vault_id));
joinable!(vaults -> accounts (account_id));

allow_tables_to_appear_in_same_query!(
    accounts,
    config,
    key_manager_states,
    outputs,
    proofs,
    substates,
    transactions,
    vaults,
);
