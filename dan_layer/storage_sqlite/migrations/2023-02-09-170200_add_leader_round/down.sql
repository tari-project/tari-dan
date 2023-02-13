ALTER TABLE last_voted_heights DROP COLUMN leader_round bigint;
ALTER TABLE leader_proposals DROP COLUMN leader_round bigint;
ALTER TABLE nodes DROP COLUMN leader_round bigint;
