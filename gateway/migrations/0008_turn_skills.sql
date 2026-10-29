-- Skills the user chose for a turn with "/name" in the composer. Their instructions are added to
-- the request codex receives; the stored message keeps only what the user typed.
ALTER TABLE turns ADD COLUMN skill_ids uuid[] NOT NULL DEFAULT '{}' CHECK (cardinality(skill_ids) <= 10);
