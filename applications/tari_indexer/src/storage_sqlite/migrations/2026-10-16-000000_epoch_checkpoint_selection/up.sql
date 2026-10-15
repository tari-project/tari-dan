-- The shard range of each checkpoint's shard group, so that the checkpoints of the shard group containing a given
-- shard can be selected. `shard_group` holds the range as "start-end".
alter table epoch_checkpoints add column shard_start integer not null default 0;
alter table epoch_checkpoints add column shard_end integer not null default 0;
-- Whether the checkpoint carries its end-of-epoch header's metadata preimage, which names the next epoch's validator
-- set. Only such checkpoints are served.
alter table epoch_checkpoints add column has_header_metadata boolean not null default false;

update epoch_checkpoints
set shard_start         = cast(substr(shard_group, 1, instr(shard_group, '-') - 1) as integer),
    shard_end           = cast(substr(shard_group, instr(shard_group, '-') + 1) as integer),
    has_header_metadata = json_extract(json_data, '$.header_metadata') is not null;
