create table high_qcs (
    id integer not null primary key autoincrement,
    shard_id blob  not null,
    height integer not null,
    is_highest integer not null,
    qc_json text not null
);
