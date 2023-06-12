-- NFTs
CREATE TABLE non_fungible_tokens
(
    id               INTEGER  NOT NULL PRIMARY KEY AUTOINCREMENT,
    vault_id         INTEGER  NOT NULL REFERENCES vaults (id),
    nft_id           TEXT     NOT NULL,
    metadata         TEXT     NOT NULL,
    is_burned        BOOLEAN     NOT NULL,  
    created_at       DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at       DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX nfts_uniq_address ON non_fungible_tokens (nft_id);
