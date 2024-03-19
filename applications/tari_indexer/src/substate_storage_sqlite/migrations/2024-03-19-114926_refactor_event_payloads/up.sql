-- All key-value pairs in event payloads
create table event_payloads
(
    id               integer not NULL primary key AUTOINCREMENT,
    payload_key      text not NULL,
    payload_value    text not NULL,
    event_id         integer not NULL,

    FOREIGN KEY (event_id) REFERENCES events (id)
);

-- A payload key should be unique by event
create unique index unique_event_payload_keys on event_payloads (event_id, payload_key);

-- Index for faster retrieval of all payloads of an event
create index event_payloads_index on event_payloads (event_id);

-- Index for faster scan queries by key and value
create index event_payloads_key_value_index on event_payloads (payload_key, payload_value);