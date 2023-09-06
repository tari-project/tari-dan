--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

-- Your SQL goes here
CREATE TABLE transaction_pool_history
(
    history_id       INTEGER PRIMARY KEY,
    id               integer   not null,
    transaction_id   text      not null,
    involved_shards  text      not null,
    original_decision text      not null,
    local_decision  text      null,
    remote_decision  text      null,
    evidence         text      not null,
    transaction_fee              bigint    not null,
    leader_fee              bigint    not null,
    stage            text      not null,
    pending_stage            text      null,
    is_ready         boolean   not null,
    updated_at    timestamp NOT NULL,
    created_at       timestamp NOT NULL,
    change_time      DATETIME DEFAULT(STRFTIME('%Y-%m-%d %H:%M:%f', 'NOW'))
);

CREATE TRIGGER copy_transaction_pool_history
    AFTER UPDATE
    ON transaction_pool
    FOR EACH ROW
BEGIN
    INSERT INTO transaction_pool_history (id,
                                          transaction_id,
                                          involved_shards,
                                          original_decision,
                                          local_decision,
                                          remote_decision,
                                          evidence,
                                          transaction_fee,
                                          leader_fee,
                                          stage,
                                          pending_stage,
                                          is_ready,
                                          updated_at,
                                          created_at)
    VALUES  (
             OLD.id,
             OLD.transaction_id,
             OLD.involved_shards,
             OLD.original_decision,
             OLD.local_decision,
             OLD.remote_decision,
             OLD.evidence,
             OLD.transaction_fee,
             OLD.leader_fee,
             OLD.stage,
             OLD.pending_stage,
             OLD.is_ready,
             OLD.updated_at,
             OLD.created_at
             );
END;
