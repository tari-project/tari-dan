CREATE TABLE committees
(
    id                INTEGER PRIMARY KEY autoincrement NOT NULL,
    validator_node_id INTEGER                           NOT NULL,
    epoch             BIGINT                            NOT NULL,
    shard_start       INTEGER                           NOT NULL,
    shard_end         INTEGER                           NOT NULL,
    FOREIGN KEY (validator_node_id) REFERENCES validator_nodes (id)
);

CREATE INDEX committees_validator_node_id_epoch_index ON committees (validator_node_id, epoch);
