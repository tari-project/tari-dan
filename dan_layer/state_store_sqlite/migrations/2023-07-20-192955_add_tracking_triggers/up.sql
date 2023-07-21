--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause

-- Your SQL goes here
CREATE TABLE transaction_pool_history
(
    history_id       INTEGER PRIMARY KEY,
    id               integer   not null,
    transaction_id   text      not null,
    involved_shards  text      not null,
    overall_decision text      not null,
    evidence         text      not null,
    fee              bigint    not null,
    stage            text      not null,
    is_ready         boolean   not null,
    created_at       timestamp NOT NULL,
    change_time      DATETIME DEFAULT(STRFTIME('%Y-%m-%d %H:%M:%f', 'NOW'))
);

CREATE TRIGGER copy_transaction_pool_history
    AFTER UPDATE
    ON transaction_pool
    FOR EACH ROW
BEGIN
    INSERT INTO transaction_pool_history (id, transaction_id,
                                          involved_shards,
                                          overall_decision,
                                          evidence,
                                          fee,
                                          stage,
                                          is_ready,
                                          created_at)
    VALUES (OLD.id,
            OLD.transaction_id,
            OLD.involved_shards,
            OLD.overall_decision,
            OLD.evidence,
            OLD.fee,
            OLD.stage,
            OLD.is_ready,
            OLD.created_at);
END;
