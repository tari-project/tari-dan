// @generated automatically by Diesel CLI.

diesel::table! {
    substates (id) {
        id -> Integer,
        address -> Text,
        version -> BigInt,
        data -> Text,
    }
}

diesel::allow_tables_to_appear_in_same_query!(substates,);
