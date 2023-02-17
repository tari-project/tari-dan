
create table substates
(
    id                      integer   not NULL primary key AUTOINCREMENT,
    address                 text      not NULL,
    version                 bigint    not NULL,
    data                    text      not NULL
);

create unique index uniq_substates_address on substates (address);