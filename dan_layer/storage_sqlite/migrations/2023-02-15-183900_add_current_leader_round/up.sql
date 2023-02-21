CREATE TABLE current_leader_states
(
    id           integer   NOT NULL PRIMARY KEY AUTOINCREMENT,
    payload_id   blob      NOT NULL,
    shard_id     blob      NOT NULL,
    leader_round bigint    NOT NULL,
    leader       blob      NOT NULL,
    timestamp    timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX current_leader_states_index_payload_id_shard_id ON current_leader_states (payload_id, shard_id);
