--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

alter table accounts
add column is_default boolean not null default 0;
