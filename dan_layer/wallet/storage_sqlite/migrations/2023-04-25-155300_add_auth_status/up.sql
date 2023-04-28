--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

-- Auth token, we don't store the auth token, the token in this table is the jwt token that is granted when user accepts the auth login request.
CREATE TABLE auth_status
(
    id           INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    user_decided BOOLEAN                           NOT NULL,
    granted      BOOLEAN                           NOT NULL,
    token        TEXT                                  NULL
);
