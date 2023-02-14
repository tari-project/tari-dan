ALTER TABLE last_voted_heights ADD leader_round bigint DEFAULT 0 NOT NULL;
ALTER TABLE leader_proposals ADD leader_round bigint DEFAULT 0 NOT NULL;
ALTER TABLE nodes ADD leader_round bigint DEFAULT 0 NOT NULL;
