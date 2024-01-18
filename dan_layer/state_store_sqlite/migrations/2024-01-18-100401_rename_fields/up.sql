
alter table substates
    rename column address to substate_id;

alter table substates
    rename column shard_id to address;

alter table locked_outputs
    rename column shard_id to substate_address;

-- to rename an index we need to drop it and crete it again
drop index substates_uniq_shard_id;
create unique index substates_uniq_address on substates (address);

drop index substates_uniq_shard_id;
create unique index locked_outputs_uniq_idx_substate_address on locked_outputs (substate_address);
    