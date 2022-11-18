--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

create table high_qcs (
                          id integer not null primary key autoincrement,
                          shard_id blob  not null,
                          height bigint not null,
                          qc_json text not null
);
