ALTER TABLE validator_nodes
    ADD COLUMN fee_claim_public_key BLOB NOT NULL DEFAULT 'invalid';
