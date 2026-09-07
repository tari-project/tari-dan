drop table substate_cache_invalidations;

create table substate_cache_invalidations
(
    substate_id    text   not null primary key,
    state_version  bigint not null,
    invalidated_at bigint not null
);

create index idx_substate_cache_invalidations_expiry on substate_cache_invalidations (invalidated_at);
