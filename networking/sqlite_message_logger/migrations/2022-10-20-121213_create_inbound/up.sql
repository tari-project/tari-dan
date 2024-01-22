--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

create table inbound_messages
(
    id           integer primary key autoincrement,
    from_pubkey  text      not null,
    message_type text      not null,
    message_json text      not null,
    message_tag  text      not null,
    received_at  timestamp not null default current_timestamp
);
