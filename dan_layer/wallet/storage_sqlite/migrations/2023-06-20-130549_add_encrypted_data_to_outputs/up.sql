-- Copyright 2022 The Tari Project
-- SPDX-License-Identifier: BSD-3-Clause

ALTER TABLE outputs
    ADD COLUMN encrypted_data blob NOT NULL DEFAULT '';
