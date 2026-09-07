-- Record the substate version a transition showed, alongside the shard version it committed at.
--
-- The shard version answers "did this fetch start before the transition", which catches a fetch the
-- transition overtook. It says nothing about a fetch that starts afterwards and lands on a committee
-- member that is behind: that member answers with a version this indexer has already watched the
-- substate pass, and with no cached row left to rank against - the transition deleted it - the
-- answer is installed as the head. Holding the version the stream showed gives that write something
-- to be refused by.
--
-- The journal is derived from the stream and spans a fetch, so it is recreated rather than migrated.
drop table substate_cache_invalidations;

create table substate_cache_invalidations
(
    substate_id      text    not null primary key,
    state_version    bigint  not null,
    -- The substate version the transition showed: the version created, or the version destroyed.
    substate_version integer not null,
    invalidated_at   bigint  not null
);

create index idx_substate_cache_invalidations_expiry on substate_cache_invalidations (invalidated_at);
