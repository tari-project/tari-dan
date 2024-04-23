CREATE TABLE committees
(
    id               INTEGER PRIMARY KEY autoincrement NOT NULL,
    public_key       BLOB,
    epoch            BIGINT                            NOT NULL,
    committee_bucket BIGINT                            NOT NULL
--  This is strange: public key always errors with foreign key mismatches on insert
--  even though it exists in the validator node table. Tested with clean db etc.
--  Might be because of the BLOB type and/or some bug in diesel.
--     FOREIGN KEY (public_key) REFERENCES validator_nodes (public_key)
);

CREATE INDEX committees_epoch_index ON committees (epoch);
