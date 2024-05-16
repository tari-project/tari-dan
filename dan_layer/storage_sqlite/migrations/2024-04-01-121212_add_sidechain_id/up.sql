-- Your SQL goes here
ALTER TABLE validator_nodes
    ADD COLUMN sidechain_id BLOB NOT NULL;

-- drop index validator_nodes_public_key_uniq_idx;

-- create unique index validator_nodes_public_key_uniq_idx on validator_nodes (public_key, sidechain_id);