CREATE TABLE committees
(
    id               INTEGER PRIMARY KEY autoincrement NOT NULL,
    validator_node_id     INTEGER                              NOT NULL,
    epoch            BIGINT                            NOT NULL,
    committee_bucket BIGINT                            NOT NULL,
    FOREIGN KEY (validator_node_id) REFERENCES validator_nodes (id)
);

CREATE INDEX committees_epoch_index ON committees (epoch);
