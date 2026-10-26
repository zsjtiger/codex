import { useEffect, useState } from "react";
import { scheduled as scheduledApi, type TaskInput } from "../api/resources";
import type { Frequency, Project, ScheduledTask } from "../api/types";

const weekdays = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

function describe(task: ScheduledTask): string {
  const time = `${task.time_of_day} UTC`;
  switch (task.frequency) {
    case "hourly":
      return `Hourly at :${task.time_of_day.slice(3)}`;
    case "daily":
      return `Daily at ${time}`;
    case "weekdays":
      return `Weekdays at ${time}`;
    case "weekly":
      return `${weekdays[(task.weekday ?? 1) - 1]}s at ${time}`;
  }
}

interface ScheduledPageProps {
  workspaceId: string;
  projects: Project[];
  canEdit: (task: ScheduledTask) => boolean;
  onOpenThread: (threadId: string) => void;
  onBack: () => void;
  onError: (message: string) => void;
}

const emptyForm = { name: "", prompt: "", frequency: "daily" as Frequency, time_of_day: "08:00", weekday: 1, project_id: "" };

/** Scheduled tasks start a new chat with their prompt on a schedule (times are UTC). */
export function ScheduledPage({ workspaceId, projects, canEdit, onOpenThread, onBack, onError }: ScheduledPageProps) {
  const [items, setItems] = useState<ScheduledTask[]>([]);
  const [creating, setCreating] = useState(false);
  const [form, setForm] = useState(emptyForm);

  const reload = () =>
    scheduledApi
      .list(workspaceId)
      .then(setItems)
      .catch((err) => onError(err instanceof Error ? err.message : "Could not load scheduled tasks."));

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  const create = async () => {
    const body: TaskInput = {
      name: form.name.trim(),
      prompt: form.prompt.trim(),
      frequency: form.frequency,
      time_of_day: form.time_of_day,
      weekday: form.frequency === "weekly" ? form.weekday : null,
      project_id: form.project_id || null,
    };
    try {
      await scheduledApi.create(workspaceId, body);
      setCreating(false);
      setForm(emptyForm);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not create the task.");
    }
  };

  const act = async (fn: () => Promise<unknown>, failure: string) => {
    try {
      await fn();
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : failure);
    }
  };

  const runNow = async (task: ScheduledTask) => {
    try {
      const { thread_id } = await scheduledApi.runNow(task.id);
      await reload();
      onOpenThread(thread_id);
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not start the run.");
    }
  };

  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>
          ← Back to chat
        </button>
        <h1>Scheduled tasks</h1>

        <div className="customize-toolbar">
          <span />
          {!creating && (
            <button className="add-button" onClick={() => setCreating(true)}>
              New task
            </button>
          )}
        </div>

        {creating && (
          <form
            className="form-card"
            onSubmit={(event) => {
              event.preventDefault();
              void create();
            }}
          >
            <label className="field">
              <span>Name</span>
              <input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} maxLength={100} required />
            </label>
            <label className="field">
              <span>Prompt</span>
              <textarea value={form.prompt} onChange={(e) => setForm({ ...form, prompt: e.target.value })} rows={5} maxLength={100000} required />
            </label>
            <div className="row">
              <label className="field">
                <span>Repeats</span>
                <select value={form.frequency} onChange={(e) => setForm({ ...form, frequency: e.target.value as Frequency })}>
                  <option value="hourly">Hourly</option>
                  <option value="daily">Daily</option>
                  <option value="weekdays">Weekdays</option>
                  <option value="weekly">Weekly</option>
                </select>
              </label>
              {form.frequency === "weekly" && (
                <label className="field">
                  <span>Day</span>
                  <select value={form.weekday} onChange={(e) => setForm({ ...form, weekday: Number(e.target.value) })}>
                    {weekdays.map((day, index) => (
                      <option key={day} value={index + 1}>
                        {day}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              <label className="field">
                <span>{form.frequency === "hourly" ? "Minute past the hour (UTC)" : "Time (UTC)"}</span>
                <input
                  type={form.frequency === "hourly" ? "text" : "time"}
                  value={form.frequency === "hourly" ? form.time_of_day.slice(3) : form.time_of_day}
                  onChange={(e) =>
                    setForm({ ...form, time_of_day: form.frequency === "hourly" ? `00:${e.target.value.padStart(2, "0")}` : e.target.value })
                  }
                  pattern={form.frequency === "hourly" ? "[0-5]?[0-9]" : undefined}
                  required
                />
              </label>
              <label className="field">
                <span>Project</span>
                <select value={form.project_id} onChange={(e) => setForm({ ...form, project_id: e.target.value })}>
                  <option value="">No project</option>
                  {projects.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <div className="form-actions">
              <button type="button" onClick={() => setCreating(false)}>
                Cancel
              </button>
              <button className="primary" type="submit" disabled={!form.name.trim() || !form.prompt.trim()}>
                Create task
              </button>
            </div>
          </form>
        )}

        {items.length === 0 && !creating && <p className="empty-state">No scheduled tasks yet.</p>}
        <div className="list">
          {items.map((task) => (
            <div key={task.id} className="list-row">
              <div className="list-main">
                <div className="list-title">{task.name}</div>
                <div className="list-sub">
                  {task.enabled ? "Active" : "Paused"} · {describe(task)}
                  {task.enabled && ` · next ${new Date(task.next_run_at).toLocaleString()}`}
                </div>
                <div className="list-sub clamp">{task.prompt}</div>
              </div>
              {canEdit(task) && (
                <div className="list-actions">
                  <button onClick={() => void runNow(task)}>Run now</button>
                  <button onClick={() => void act(() => scheduledApi.update(task.id, { enabled: !task.enabled }), "Could not update the task.")}>
                    {task.enabled ? "Pause" : "Resume"}
                  </button>
                  <button onClick={() => void act(() => scheduledApi.remove(task.id), "Could not delete the task.")}>Delete</button>
                </div>
              )}
            </div>
          ))}
        </div>
      </div>
    </main>
  );
}
