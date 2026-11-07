-- Attachments sent with a turn, so the conversation can show them and open them.
ALTER TABLE turns ADD COLUMN attachment_ids uuid[] NOT NULL DEFAULT '{}' CHECK (cardinality(attachment_ids) <= 20);
