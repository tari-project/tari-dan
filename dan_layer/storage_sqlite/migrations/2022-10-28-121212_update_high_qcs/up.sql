--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

create unique index high_qcs_index_shard_id_height on high_qcs (shard_id, height);
