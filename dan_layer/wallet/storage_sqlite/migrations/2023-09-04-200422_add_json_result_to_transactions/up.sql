--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

ALTER TABLE transactions
    ADD COLUMN json_result TEXT NULL;
