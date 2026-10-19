DROP INDEX utxos_resource_shard_state_version_epoch_idx;
CREATE INDEX utxos_resource_state_version_shard_epoch_idx ON utxos (resource_address, state_version, shard, epoch);
