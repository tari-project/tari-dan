-- NFTs
CREATE TABLE non_fungible_tokens
(
    id               INTEGER  NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id       INTEGER  NOT NULL REFERENCES accounts (id),
    resource_address TEXT     NOT NULL,
    token_symbol     TEXT     NOT NULL,
    nft_id           BIGINT   NOT NULL,
    created_at       DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at       DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX nfts_uniq_address ON non_fungible_tokens (resource_address);
