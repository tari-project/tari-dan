-- Key Manager

CREATE TABLE key_manager_states
(
    id          INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    branch_seed TEXT                              NOT NULL,
    `index`     BIGINT                            NOT NULL,
    created_at  DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX key_manager_states_uniq_branch_seed on key_manager_states (branch_seed);

-- Config

CREATE TABLE config
(
    id           INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    key          TEXT                              NOT NULL,
    value        TEXT                              NOT NULL,
    is_encrypted BOOLEAN                           NOT NULL,
    created_at   DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at   DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX config_uniq_key on config (key);

-- Transaction
CREATE TABLE transactions
(
    id             INTEGER  NOT NULL PRIMARY KEY AUTOINCREMENT,
    hash           TEXT     NOT NULL,
    instructions   TEXT     NOT NULL,
    signature      TEXT     NOT NULL,
    sender_address TEXT     NOT NULL,
    fee            BIGINT   NOT NULL,
    meta           TEXT     NOT NULL,
    result         TEXT     NULL,
    qcs            TEXT     NULL,
    status         TEXT     NOT NULL,
    created_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX transactions_uniq_hash ON transactions (hash);
CREATE INDEX transactions_idx_status ON transactions (status);

-- Substates
CREATE TABLE substates
(
    id               INTEGER  NOT NULL PRIMARY KEY AUTOINCREMENT,
    module_name      TEXT     NULL,
    address          TEXT     NOT NULL,
    parent_address   TEXT     NULL,
    version          INTEGER  NOT NULL,
    transaction_hash TEXT     NOT NULL,
    template_address TEXT     NULL,
    created_at       DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX substates_idx_transaction_hash ON substates (transaction_hash);
CREATE UNIQUE INDEX substates_uniq_address ON substates (address);

-- Accounts
CREATE TABLE accounts
(
    id              INTEGER  NOT NULL PRIMARY KEY AUTOINCREMENT,
    name            TEXT     NOT NULL,
    address         TEXT     NOT NULL,
    owner_key_index BIGINT   NOT NULL,
    created_at      DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at      DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX accounts_uniq_address ON accounts (address);
CREATE UNIQUE INDEX accounts_uniq_name ON accounts (name);
