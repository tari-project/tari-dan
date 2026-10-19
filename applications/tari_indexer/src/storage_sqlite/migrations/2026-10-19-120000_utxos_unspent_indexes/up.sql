-- Finds a UTXO by its commitment, as the list cursor and marking an output spent do.
CREATE INDEX utxos_resource_commitment_idx ON utxos (resource_address, commitment);
-- A resource's unspent UTXOs in rowid order, so that listing them pages from a cursor without a sort. Queries
-- must repeat this condition as SQL literals for SQLite to use the index.
CREATE INDEX utxos_resource_unspent_idx ON utxos (resource_address) WHERE is_spent = false AND is_burnt = false;
