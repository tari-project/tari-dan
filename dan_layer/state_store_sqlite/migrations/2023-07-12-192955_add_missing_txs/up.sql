-- Your SQL goes here
CREATE TABLE block_missing_txs
(
    id              integer   not NULL PRIMARY KEY AUTOINCREMENT,
    transaction_ids text      not NULL,
    created_at      timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    block_id        text      not NULL
);

CREATE TABLE missing_tx
(
    id              integer   not NULL primary key AUTOINCREMENT,
    transaction_id  text      not NULL,
    created_at      timestamp not NULL DEFAULT CURRENT_TIMESTAMP,
    block_id        text      not NULL
);
