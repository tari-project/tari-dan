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

diesel::allow_tables_to_appear_in_same_query!(non_fungible_indexes, substates,);
