// @generated automatically by Diesel CLI.

diesel::table! {
    non_fungible_indexes (id) {
        id -> Integer,
        resource_address -> Text,
        idx -> Integer,
        non_fungible_address -> Text,
    }
}

diesel::table! {
    substates (id) {
        id -> Integer,
        address -> Text,
        version -> BigInt,
        data -> Text,
        transaction_hash -> Nullable<Binary>,
    }
}

diesel::table! {
    events (id) {
        id -> Integer,
        template_address -> Text,
        tx_hash -> Text,
        topic -> Text,
        version -> Integer,
        component_address -> Nullable<Text>,
    }
}

diesel::table! {
    event_payloads (id) {
        id -> Integer,
        payload_key -> Text,
        payload_value -> Text,
        event_id -> Integer,
    }
}

diesel::joinable!(event_payloads -> events (event_id));

diesel::allow_tables_to_appear_in_same_query!(substates, non_fungible_indexes, events, event_payloads);
