--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

create table transaction_results (
    id integer not null primary key autoincrement,
    payload_id blob  not null,
    result_bytes blob not null
);

create unique index transaction_results_payload_id_index on transaction_results (payload_id);