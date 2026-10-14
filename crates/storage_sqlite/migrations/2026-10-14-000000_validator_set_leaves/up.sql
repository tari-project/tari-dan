-- NULL until the epoch manager stores the hash, which it does each time it assigns the epoch's committees.
ALTER TABLE epochs ADD COLUMN validator_set_hash blob NULL;

-- The committee leaf hash of each shard group in an epoch's validator set tree.
CREATE TABLE validator_set_leaves
(
    epoch       bigint  not null references epochs (epoch),
    shard_start integer not null,
    shard_end   integer not null,
    leaf_hash   blob    not null,
    primary key (epoch, shard_start, shard_end)
);
