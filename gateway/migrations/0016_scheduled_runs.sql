-- One row per scheduled run, so the task page can show its history.
CREATE TABLE scheduled_task_runs (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id    uuid NOT NULL REFERENCES scheduled_tasks (id) ON DELETE CASCADE,
    thread_id  uuid REFERENCES threads (id) ON DELETE SET NULL,
    started_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX scheduled_task_runs_task_idx ON scheduled_task_runs (task_id, started_at DESC);
