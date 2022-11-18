alter table high_qcs add column identity blob not null ;
drop index high_qcs_index_shard_id_height;
create unique index high_qcs_index_shard_id_height on high_qcs (shard_id, height, identity);
