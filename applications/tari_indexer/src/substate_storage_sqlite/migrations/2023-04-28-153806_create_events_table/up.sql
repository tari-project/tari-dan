-- Event data
create table events
(
    id               integer not NULL primary key AUTOINCREMENT,
    template_address text    not NULL,
    tx_hash          text    not NULL,
    topic            text    not NULL,
    payload          text    not NULL        
);


-- An event should be uniquely identified by its template_address, tx_hash and topic
create unique index unique_events_indexer on events (template_address, tx_hash, topic);

-- DB index for faster collection scan queries
create index events_indexer on events (template_address, tx_hash);
