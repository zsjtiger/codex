-- The user's rating of a reply: "up", "down", or unset.
ALTER TABLE turns ADD COLUMN feedback text CHECK (feedback IN ('up', 'down'));
