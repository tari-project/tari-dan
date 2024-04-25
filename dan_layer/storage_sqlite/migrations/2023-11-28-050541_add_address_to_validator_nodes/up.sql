-- Your SQL goes here
ALTER TABLE validator_nodes
    ADD COLUMN address TEXT NOT NULL DEFAULT 'invalid';
