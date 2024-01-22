ALTER TABLE foreign_proposals
    RENAME COLUMN mined_at TO proposed_height;

UPDATE foreign_proposals
    SET state = 'Proposed'
    WHERE state = 'Mined';