-- Where a sign-in's provider callback must go: the runtime host and port that codex's local
-- OAuth listener is on. Keyed by the OAuth `state` value, which the provider echoes back.
CREATE TABLE oauth_callbacks (
    state       text PRIMARY KEY CHECK (length(state) BETWEEN 1 AND 500),
    target_host text NOT NULL CHECK (length(target_host) BETWEEN 1 AND 255),
    target_port integer NOT NULL CHECK (target_port BETWEEN 1 AND 65535),
    created_at  timestamptz NOT NULL DEFAULT now()
);
