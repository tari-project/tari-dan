CREATE TABLE committees
(
    id               INTEGER PRIMARY KEY autoincrement NOT NULL,
    public_key       BLOB                              NOT NULL,
    epoch            BIGINT                            NOT NULL,
    committee_bucket BIGINT                            NOT NULL,
    FOREIGN KEY (public_key) REFERENCES validator_nodes (public_key)
);

CREATE INDEX committees_epoch_index ON committees (epoch);
