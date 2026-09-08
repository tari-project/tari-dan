-- Record whether the version a transition showed is itself spent, alongside the version.
--
-- The version alone is a floor: a result below it is refused. It cannot also express that the floor
-- version is down, which a destroy with no successor leaves true - the successor is what would
-- otherwise raise the floor past it. Without the provenance a lagging committee member's `Up` at the
-- destroyed version is admitted as the live head and a spent substate is served as spendable, while
-- a `Down` at that same version is a legitimate head and must still be admitted.
--
-- The journal is derived from the stream and spans a fetch, so it is recreated rather than migrated.
drop table substate_cache_invalidations;

create table substate_cache_invalidations
(
    substate_id      text    not null primary key,
    state_version    bigint  not null,
    -- The substate version the transition showed: the version created, or the version destroyed.
    substate_version integer not null,
    -- Whether `substate_version` was destroyed rather than created.
    spent            boolean not null,
    invalidated_at   bigint  not null
);

create index idx_substate_cache_invalidations_expiry on substate_cache_invalidations (invalidated_at);
