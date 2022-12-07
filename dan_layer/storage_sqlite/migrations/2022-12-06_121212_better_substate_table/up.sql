--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

drop table substates;

create table substates (
                           id integer not null primary key AUTOINCREMENT,
                           shard_id blob not NULL,
                           -- To be deleted in future
                           version bigint not NULL,
                           data text not null,
                           created_by_payload_id blob not null,
                           created_justify text NOt null,
                           created_node_hash blob NOT NULL,
                           created_height bigint NOT NULL,
                           destroyed_by_payload_id blob null,
                           destroyed_justify text null,
                           destroyed_node_hash blob NULL,
                           destroyed_height bigint NULL,
                           created_timestamp  timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
                           destroyed_timestamp timestamp NULL
);

create table shard_pledges (
    id integer not null primary key AUTOINCREMENT,
    shard_id blob not NULL,
    created_height bigint NOT NULL,
    pledged_to_payload_id blob not NULL,
    is_active boolean not NULL,
    completed_by_tree_node_hash blob NULL,
    abandoned_by_tree_node_hash blob NULL,
    timestamp timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_timestamp timestamp NULL
)
