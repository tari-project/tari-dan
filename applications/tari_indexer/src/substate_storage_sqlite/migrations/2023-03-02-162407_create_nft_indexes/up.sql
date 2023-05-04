-- all the indexes in NFT resources
create table non_fungible_indexes
(
    id                   integer not NULL primary key AUTOINCREMENT,
    resource_address     text    not NULL,
    idx                  integer not NULL,
    non_fungible_address text    not NULL,
    FOREIGN KEY (resource_address) REFERENCES substates (address),
    FOREIGN KEY (non_fungible_address) REFERENCES substates (address)
);

-- A list can only have one single item at any specific position
create unique index uniq_nft_indexes on non_fungible_indexes (resource_address, idx);

-- DB index for faster collection scan queries
create index nft_indexes_resource on non_fungible_indexes (resource_address, idx);