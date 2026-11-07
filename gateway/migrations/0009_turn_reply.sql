-- The assistant's text for a turn, kept after the live stream ends so a reload still shows it.
ALTER TABLE turns ADD COLUMN reply text CHECK (length(reply) <= 200000);
