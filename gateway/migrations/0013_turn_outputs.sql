-- Files the agent produced for a turn (saved under the turn's output folder), shown beside the reply.
ALTER TABLE turns ADD COLUMN output_attachment_ids uuid[] NOT NULL DEFAULT '{}' CHECK (cardinality(output_attachment_ids) <= 10);
