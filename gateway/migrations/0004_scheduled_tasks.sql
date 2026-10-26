-- A prompt that runs on a schedule. Each run starts a new thread with the prompt as its first turn.
CREATE TABLE scheduled_tasks (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id   uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name           text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    prompt         text NOT NULL CHECK (length(prompt) BETWEEN 1 AND 100000),
    frequency      text NOT NULL CHECK (frequency IN ('hourly', 'daily', 'weekdays', 'weekly')),
    -- "HH:MM" in UTC. For hourly tasks, the minute past each hour.
    time_of_day    text NOT NULL CHECK (time_of_day ~ '^([01][0-9]|2[0-3]):[0-5][0-9]$'),
    -- ISO weekday, 1 = Monday, for weekly tasks.
    weekday        smallint CHECK (weekday BETWEEN 1 AND 7),
    environment_id uuid REFERENCES environments (id) ON DELETE SET NULL,
    project_id     uuid REFERENCES projects (id) ON DELETE SET NULL,
    enabled        boolean NOT NULL DEFAULT true,
    next_run_at    timestamptz NOT NULL,
    last_run_at    timestamptz,
    last_thread_id uuid REFERENCES threads (id) ON DELETE SET NULL,
    created_by     uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now(),
    CHECK (frequency <> 'weekly' OR weekday IS NOT NULL)
);
CREATE INDEX scheduled_tasks_due_idx ON scheduled_tasks (next_run_at) WHERE enabled;
CREATE INDEX scheduled_tasks_workspace_idx ON scheduled_tasks (workspace_id);
