alter table substates rename to substates_old;

create table substates
(
    id               integer   not NULL primary key AUTOINCREMENT,
    address          text      not NULL,
    version          int       not NULL,
    data             text      not NULL,
    template_address text      NULL,
    module_name      text      NULL,
    updated_at       timestamp not null default current_timestamp,
    created_at       timestamp not null default current_timestamp
);

insert into substates (id, address, version, data, template_address, module_name, updated_at, created_at)
select id, address, version, data, template_address, module_name, updated_at, created_at
from substates_old;

drop table substates_old;

create unique index uniq_substates_address on substates (address);

alter table substate_transitions rename to substate_transitions_old;

create table substate_transitions
(
    id            integer   not NULL primary key AUTOINCREMENT,
    shard         int       not NULL,
    state_version bigint    not NULL,
    epoch         bigint    not NULL,
    substate_id   text      not NULL,
    version       int       not NULL,
    substate_type text      not NULL,
    is_up         bool      not NULL,
    value_hash    text      NULL,
    created_at    timestamp not null default current_timestamp
);

insert into substate_transitions (id, shard, state_version, epoch, substate_id, version, substate_type, is_up,
                                 value_hash, created_at)
select id,
       shard,
       state_version,
       epoch,
       substate_id,
       version,
       substate_type,
       is_up,
       value_hash,
       created_at
from substate_transitions_old;

drop table substate_transitions_old;

create unique index substate_transitions_substate_id_version_uniq on substate_transitions (substate_id, version, is_up);
create index substate_transitions_shard_state_version_idx on substate_transitions (shard, state_version);

alter table utxos rename to utxos_old;

create table utxos
(
    id               integer   not NULL primary key AUTOINCREMENT,
    commitment       text      not NULL,
    public_nonce     text      not NULL,
    version          int       not NULL,
    resource_address text      not NULL,
    shard            int       not NULL,
    state_version    bigint    not NULL,
    output           blob      NULL,
    utxo_tag         int       not NULL,
    epoch            bigint    not NULL,
    is_spent         boolean   not NULL,
    is_burnt         boolean   not NULL,
    is_frozen        boolean   not NULL,
    created_at       timestamp not null default current_timestamp
);

insert into utxos (id, commitment, public_nonce, version, resource_address, shard, state_version, output, utxo_tag,
                   epoch, is_spent, is_burnt, is_frozen, created_at)
select id,
       commitment,
       public_nonce,
       version,
       resource_address,
       shard,
       state_version,
       output,
       utxo_tag,
       epoch,
       is_spent,
       is_burnt,
       is_frozen,
       created_at
from utxos_old;

drop table utxos_old;

CREATE INDEX utxos_resource_state_version_shard_epoch_idx ON utxos (resource_address, state_version, shard, epoch);
CREATE UNIQUE INDEX utxos_resource_public_nonce_utxo_tag_uniq_partial ON utxos (resource_address, public_nonce, utxo_tag) WHERE is_spent = false;

drop table substate_cache;

create table substate_cache
(
    substate_id     text    not null primary key,
    version         integer null,
    verified        boolean not null,
    substate_result blob    not null,
    cached_at       bigint  not null
);

create index idx_substate_cache_evict on substate_cache (cached_at);

drop table substate_cache_invalidations;

create table substate_cache_invalidations
(
    substate_id      text    not null primary key,
    state_version    bigint  not null,
    substate_version integer not null,
    spent            boolean not null,
    invalidated_at   bigint  not null
);

create index idx_substate_cache_invalidations_expiry on substate_cache_invalidations (invalidated_at);
