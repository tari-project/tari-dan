--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

ALTER TABLE auth_status
    ADD COLUMN revoked BOOLEAN NOT NULL DEFAULT FALSE;
