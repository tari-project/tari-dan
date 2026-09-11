// @generated automatically by Diesel CLI.

diesel::table! {
    accounts (id) {
        id -> Integer,
        name -> Nullable<Text>,
        address -> Text,
        owner_public_key -> Text,
        view_only_key_id -> Text,
        owner_key_id -> Nullable<Text>,
        birthday_epoch -> BigInt,
        is_default -> Bool,
        is_confirmed_on_chain -> Bool,
        stealth_resources -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    account_balance_changes (id) {
        id -> Integer,
        account_id -> Integer,
        resource_id -> Integer,
        account_address -> Text,
        vault_address -> Nullable<Text>,
        vault_version -> Nullable<BigInt>,
        resource_address -> Text,
        token_symbol -> Nullable<Text>,
        divisibility -> Integer,
        source_type -> Text,
        transaction_id -> Nullable<Text>,
        revealed_before -> Text,
        revealed_after -> Text,
        confidential_before -> Text,
        confidential_after -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    authored_templates (id) {
        id -> Integer,
        author_public_key -> Text,
        address -> Text,
        name -> Text,
        abi_version -> Integer,
        functions -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    api_keys (id) {
        id -> Integer,
        name -> Text,
        key_hash -> Text,
        permissions -> Text,
        created_at -> Timestamp,
        last_used_at -> Nullable<Timestamp>,
        revoked_at -> Nullable<Timestamp>,
        expires_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    confidential_outputs (id) {
        id -> Integer,
        account_id -> Integer,
        vault_id -> Integer,
        commitment -> Text,
        value -> BigInt,
        sender_public_nonce -> Nullable<Text>,
        view_only_key_id -> Text,
        owner_key_id -> Nullable<Text>,
        public_asset_tag -> Nullable<Text>,
        memo_json -> Nullable<Text>,
        status -> Text,
        locked_at -> Nullable<Timestamp>,
        lock_id -> Nullable<Integer>,
        encrypted_data -> Binary,
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
    key_manager_imported_keys (id) {
        id -> Integer,
        label -> Text,
        encrypted_secret -> Binary,
        key_type -> Text,
        created_at -> Timestamp,
        public_key -> Text,
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
    locks (id) {
        id -> Integer,
        transaction_id -> Nullable<Text>,
        timeout_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    non_fungible_tokens (id) {
        id -> Integer,
        vault_id -> Integer,
        nft_id -> Text,
        resource_id -> Text,
        data -> Text,
        mutable_data -> Text,
        is_burnt -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    resources (id) {
        id -> Integer,
        address -> Text,
        resource_type -> Text,
        owner_rule -> Text,
        access_rules -> Text,
        token_symbol -> Nullable<Text>,
        divisibility -> Integer,
        metadata -> Text,
        total_supply -> Nullable<Text>,
        view_key -> Nullable<Text>,
        auth_hook -> Nullable<Text>,
        updated_at -> Timestamp,
        created_at -> Timestamp,
    }
}

diesel::table! {
    shard_state_versions (id) {
        id -> Integer,
        account_id -> Integer,
        resource_id -> Integer,
        shard -> Integer,
        state_version -> BigInt,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    stealth_outputs (id) {
        id -> Integer,
        owner_account_id -> Integer,
        resource_address -> Text,
        commitment -> Text,
        value -> BigInt,
        sender_public_nonce -> Text,
        status -> Text,
        locked_at -> Nullable<Timestamp>,
        lock_id -> Nullable<Integer>,
        view_only_key_id -> Text,
        owner_key_id -> Nullable<Text>,
        encrypted_data -> Binary,
        tag_byte -> Integer,
        memo_json -> Nullable<Text>,
        spend_key -> Nullable<Text>,
        condition_root -> Nullable<Text>,
        minimum_value_promise -> BigInt,
        is_burnt -> Bool,
        is_frozen -> Bool,
        is_on_chain -> Bool,
        is_condition_spendable -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    substates (id) {
        id -> Integer,
        module_name -> Nullable<Text>,
        address -> Text,
        parent_address -> Nullable<Text>,
        referenced_substates -> Text,
        template_address -> Nullable<Text>,
        created_at -> Timestamp,
        version -> BigInt,
    }
}

diesel::table! {
    transaction_accounts (id) {
        id -> Integer,
        transaction_id -> Text,
        account_id -> Integer,
        created_at -> Timestamp,
    }
}

diesel::table! {
    transactions (id) {
        id -> Integer,
        transaction_id -> Text,
        transaction_json -> Text,
        referenced_components -> Text,
        signers -> Text,
        result -> Nullable<Text>,
        qcs -> Nullable<Text>,
        final_fee -> Nullable<BigInt>,
        status -> Text,
        dry_run -> Bool,
        executed_time_ms -> Nullable<BigInt>,
        finalized_time -> Nullable<Timestamp>,
        new_account_info -> Nullable<Text>,
        invalid_reason -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    transaction_requests (id) {
        id -> Integer,
        unsigned_transaction -> Text,
        seal_signer -> Text,
        other_signers -> Text,
        signatures -> Text,
        lock_ids -> Text,
        requested_by -> Nullable<Text>,
        status -> Text,
        transaction_id -> Nullable<Text>,
        expires_at -> Timestamp,
        approved_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    utxo_process_queue (id) {
        id -> Integer,
        account_id -> Integer,
        resource_address -> Text,
        utxo_tag -> Integer,
        public_nonce -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    vault_locks (id) {
        id -> Integer,
        vault_id -> Integer,
        lock_id -> Integer,
        amount -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    vaults (id) {
        id -> Integer,
        account_id -> Integer,
        address -> Text,
        resource_address -> Text,
        resource_type -> Text,
        revealed_balance -> Text,
        confidential_balance -> Text,
        token_symbol -> Nullable<Text>,
        divisibility -> Integer,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        vault_version -> BigInt,
    }
}

diesel::table! {
    wallet_events (id) {
        id -> Nullable<Integer>,
        account_id -> Nullable<Integer>,
        event_type -> Text,
        event_data -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    webauthn_registration_passkeys (id) {
        id -> Integer,
        registration_id -> Integer,
        passkey -> Binary,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    webauthn_registrations (id) {
        id -> Integer,
        username -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    address_book (id) {
        id -> Integer,
        name -> Text,
        address -> Text,
        note -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::joinable!(confidential_outputs -> accounts (account_id));
diesel::joinable!(confidential_outputs -> vaults (vault_id));
diesel::joinable!(non_fungible_tokens -> vaults (vault_id));
diesel::joinable!(shard_state_versions -> accounts (account_id));
diesel::joinable!(shard_state_versions -> resources (resource_id));
diesel::joinable!(stealth_outputs -> accounts (owner_account_id));
diesel::joinable!(transaction_accounts -> accounts (account_id));
diesel::joinable!(utxo_process_queue -> accounts (account_id));
diesel::joinable!(vault_locks -> locks (lock_id));
diesel::joinable!(vault_locks -> vaults (vault_id));
diesel::joinable!(vaults -> accounts (account_id));
diesel::joinable!(wallet_events -> accounts (account_id));
diesel::joinable!(webauthn_registration_passkeys -> webauthn_registrations (registration_id));

diesel::allow_tables_to_appear_in_same_query!(
    account_balance_changes,
    accounts,
    address_book,
    api_keys,
    authored_templates,
    confidential_outputs,
    config,
    key_manager_imported_keys,
    key_manager_states,
    locks,
    non_fungible_tokens,
    resources,
    shard_state_versions,
    stealth_outputs,
    substates,
    transaction_accounts,
    transaction_requests,
    transactions,
    utxo_process_queue,
    vault_locks,
    vaults,
    wallet_events,
    webauthn_registration_passkeys,
    webauthn_registrations,
);
