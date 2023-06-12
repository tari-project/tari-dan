PRAGMA foreign_keys = ON;

-- Key Manager
CREATE TABLE key_manager_states
(
    id          INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    branch_seed TEXT                              NOT NULL,
    `index`     BIGINT                            NOT NULL,
    is_active   BOOLEAN                           NOT NULL,
    created_at  DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  DATETIME                          NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX key_manager_states_uniq_branch_seed_index on key_manager_states (branch_seed, `index`);

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
    id                  INTEGER  NOT NULL PRIMARY KEY AUTOINCREMENT,
    hash                TEXT     NOT NULL,
    instructions        TEXT     NOT NULL,
    signature           TEXT     NOT NULL,
    sender_public_key   TEXT     NOT NULL,
    fee_instructions    TEXT     NOT NULL,
    meta                TEXT     NOT NULL,
    result              TEXT     NULL,
    transaction_failure TEXT     NULL,
    qcs                 TEXT     NULL,
    final_fee           BIGINT   NULL,
    status              TEXT     NOT NULL,
    dry_run             BOOLEAN  NOT NULL,
    created_at          DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at          DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
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

-- Vaults
CREATE TABLE vaults
(
    id               INTEGER  NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id       INTEGER  NOT NULL REFERENCES accounts (id),
    address          TEXT     NOT NULL,
    resource_address TEXT     NOT NULL,
    resource_type    TEXT     NOT NULL,
    balance          BIGINT   NOT NULL DEFAULT 0,
    token_symbol     TEXT     NULL,
    created_at       DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at       DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX vaults_uniq_address ON vaults (address);

-- Outputs
CREATE TABLE outputs
(
    id                  INTEGER  NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id          INTEGER  NOT NULL REFERENCES accounts (id),
    vault_id            INTEGER  NOT NULL REFERENCES vaults (id),
    commitment          TEXT     NOT NULL,
    value               BIGINT   NOT NULL,
    sender_public_nonce TEXT     NULL,
    secret_key_index    BIGINT   NOT NULL,
    public_asset_tag    TEXT     NULL,
    -- Status can be "Unspent", "Spent", "Locked", "LockedUnconfirmed", "Invalid"
    status              TEXT     NOT NULL,
    locked_at           DATETIME NULL,
    locked_by_proof     INTEGER  NULL,
    created_at          DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at          DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX outputs_uniq_commitment ON outputs (commitment);
CREATE INDEX outputs_idx_account_status ON outputs (account_id, status);

-- Proofs
CREATE TABLE proofs
(
    id               INTEGER  NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id       INTEGER  NOT NULL REFERENCES accounts (id),
    vault_id         INTEGER  NOT NULL REFERENCES vaults (id),
    transaction_hash TEXT     NULL,
    created_at       DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

