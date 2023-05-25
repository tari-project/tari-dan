create table validator_nodes2
(
    id         integer primary key autoincrement not null,
    public_key blob                              not null,
    shard_key  blob                              not null,
    epoch      bigint                            not null
);

create index validator_nodes_epoch_index on validator_nodes (epoch);
