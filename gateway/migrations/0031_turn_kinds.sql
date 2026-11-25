-- A turn is a message, a code review, a compaction, or a note added to a running turn (steer).
ALTER TABLE turns ADD COLUMN kind text NOT NULL DEFAULT 'message'
    CHECK (kind IN ('message', 'review', 'compact', 'steer'));
-- The review target, as JSON, for review turns.
ALTER TABLE turns ADD COLUMN target text CHECK (length(target) <= 2000);
