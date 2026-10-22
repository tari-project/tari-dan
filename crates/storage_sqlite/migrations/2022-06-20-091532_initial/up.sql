--  // Copyright 2021. The Tari Project
--  //
--  // Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
--  // following conditions are met:
--  //
--  // 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
--  // disclaimer.
--  //
--  // 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
--  // following disclaimer in the documentation and/or other materials provided with the distribution.
--  //
--  // 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
--  // products derived from this software without specific prior written permission.
--  //
--  // THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
--  // INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
--  // DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
--  // SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
--  // SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
--  // WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
--  // USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

create table metadata
(
    key_name blob primary key not null,
    value    blob             not null
);

create table validator_nodes
(
    id                   integer primary key autoincrement not null,
    public_key           blob                              not null,
    address              text                              not null,
    shard_key            blob                              not null,
    start_epoch          bigint                            not null,
    end_epoch            bigint                            null,
    fee_claim_public_key blob                              not null,
    power                bigint                            not null
);

CREATE INDEX validator_nodes_address_index ON validator_nodes (address);

CREATE TABLE committees
(
    id                INTEGER PRIMARY KEY autoincrement NOT NULL,
    validator_node_id INTEGER                           NOT NULL,
    epoch             BIGINT                            NOT NULL,
    shard_start       INTEGER                           NOT NULL,
    shard_end         INTEGER                           NOT NULL,
    FOREIGN KEY (validator_node_id) REFERENCES validator_nodes (id) ON DELETE CASCADE
);

-- A validator holds one committee assignment per epoch.
CREATE UNIQUE INDEX committees_validator_node_id_epoch_index ON committees (validator_node_id, epoch);


create table epochs
(
    epoch              bigint primary key not null,
    epoch_hash         blob               not null,
    -- NULL until the epoch manager stores the hash, which it does each time it assigns the epoch's committees.
    validator_set_hash blob               null
);

-- The committee leaf hash of each shard group in an epoch's validator set tree.
CREATE TABLE validator_set_leaves
(
    epoch       bigint  not null references epochs (epoch),
    shard_start integer not null,
    shard_end   integer not null,
    leaf_hash   blob    not null,
    primary key (epoch, shard_start, shard_end)
);

create table bmt_cache
(
    epoch bigint primary key not null,
    bmt   blob               not null
);

CREATE TABLE layer_one_transactions
(
    id           INTEGER PRIMARY KEY autoincrement NOT NULL,
    epoch        BIGINT                            NOT NULL,
    payload_type TEXT                              NOT NULL,
    payload      TEXT                              NOT NULL,
    submitted_at DATETIME                          NULL,
    is_observed  BOOLEAN                           NOT NULL DEFAULT '0'
);

CREATE TABLE block_headers
(
    id                         INTEGER PRIMARY KEY autoincrement NOT NULL,
    epoch                      BIGINT                            NOT NULL,
    height                     BIGINT                            NOT NULL,
    block_hash                 BLOB                              NOT NULL,
    block_output_merkle_root   BLOB                              NOT NULL,
    validator_node_merkle_root BLOB                              NOT NULL,
    created_at                 DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX block_headers_block_hash_epoch_idx ON block_headers (block_hash, epoch);

