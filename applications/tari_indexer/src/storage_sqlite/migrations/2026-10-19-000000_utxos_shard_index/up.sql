-- Every read of `utxos` by state version is for one shard. With `shard` ahead of `state_version`, such a read is a
-- range within that shard's entries rather than over the whole resource's history.
DROP INDEX utxos_resource_state_version_shard_epoch_idx;
CREATE INDEX utxos_resource_shard_state_version_epoch_idx ON utxos (resource_address, shard, state_version, epoch);
