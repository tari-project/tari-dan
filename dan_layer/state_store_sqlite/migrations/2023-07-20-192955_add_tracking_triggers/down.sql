--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

-- This file should undo anything in `up.sql`

DROP TRIGGER copy_transaction_pool_history;
DROP TABLE transaction_pool_history;
