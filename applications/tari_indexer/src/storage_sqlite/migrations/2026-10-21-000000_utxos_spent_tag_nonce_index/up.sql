-- Finds a resource's spent or burnt UTXOs by public nonce and tag, as fetching UTXOs by key does to report which of
-- the requested outputs are spent. Queries must repeat this condition as SQL literals for SQLite to use the index.
CREATE INDEX utxos_resource_public_nonce_utxo_tag_spent_idx ON utxos (resource_address, public_nonce, utxo_tag) WHERE is_spent = true OR is_burnt = true;
