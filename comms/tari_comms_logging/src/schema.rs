table! {
    inbound_messages (id) {
        id -> Nullable<Integer>,
        from_pubkey -> Binary,
        message_type -> Text,
        message_json -> Text,
        message_tag -> Text,
        received_at -> Timestamp,
    }
}

table! {
    outbound_messages (id) {
        id -> Nullable<Integer>,
        destination_type -> Text,
        destination_pubkey -> Binary,
        message_type -> Text,
        message_json -> Text,
        message_tag -> Text,
        sent_at -> Timestamp,
    }
}

allow_tables_to_appear_in_same_query!(inbound_messages, outbound_messages,);
