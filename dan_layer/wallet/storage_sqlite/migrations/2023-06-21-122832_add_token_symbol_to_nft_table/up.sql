--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

ALTER TABLE non_fungible_tokens
    ADD COLUMN token_symbol TEXT NOT NULL;
