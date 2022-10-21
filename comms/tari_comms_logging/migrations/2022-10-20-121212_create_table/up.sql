--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

create table outbound_messages
(
    id                 integer primary key autoincrement,
    destination_type   text      not null,
    destination_pubkey blob      not null,
    message_type text not null,
    message_json       text      not null,
    sent_at            timestamp not null default current_timestamp
);
