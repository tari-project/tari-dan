--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

ALTER TABLE payloads
    ADD COLUMN timestamp BIGINT NOT NULL DEFAULT 0;

