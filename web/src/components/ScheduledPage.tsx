import { useEffect, useState } from "react";
import { scheduled as scheduledApi, type TaskInput } from "../api/resources";
import type { Frequency, Project, ScheduledRun, ScheduledTask } from "../api/types";
import { PlusIcon, SearchIcon } from "./icons";

const weekdays = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/** How a task repeats, in the times the scheduler uses (UTC). */
function describeRepeat(task: ScheduledTask): string {
  switch (task.frequency) {
    case "hourly":
      return `Hourly at :${task.time_of_day.slice(3)} UTC`;
    case "daily":
      return `Daily at ${task.time_of_day} UTC`;
    case "weekdays":
      return `Weekdays at ${task.time_of_day} UTC`;
    case "weekly":
      return `${weekdays[(task.weekday ?? 1) - 1]}s at ${task.time_of_day} UTC`;
  }
}

/** "Today at 8:05 AM", "Yesterday at …", "Tomorrow at …" or a date, in the viewer's time zone. */
function whenLabel(iso: string): string {
  const date = new Date(iso);
  const now = new Date();
  const time = date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  const offset = (days: number) => {
    const d = new Date(now);
    d.setDate(now.getDate() + days);
    return d.toDateString();
  };
  const day = date.toDateString();
  if (day === now.toDateString()) return `Today at ${time}`;
  if (day === offset(-1)) return `Yesterday at ${time}`;
  if (day === offset(1)) return `Tomorrow at ${time}`;
  return `${date.toLocaleDateString(undefined, { month: "short", day: "numeric" })} at ${time}`;
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

/** Scheduled tasks start a new chat with their prompt on a schedule. Times are UTC. */
export function ScheduledPage({ workspaceId, projects, canEdit, onOpenThread, onBack, onError }: ScheduledPageProps) {
  const [items, setItems] = useState<ScheduledTask[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [editingTask, setEditingTask] = useState<ScheduledTask | null>(null);
  const [form, setForm] = useState(emptyForm);
  const [query, setQuery] = useState("");
  const [showSearch, setShowSearch] = useState(false);
  const [runs, setRuns] = useState<ScheduledRun[]>([]);
  const [busy, setBusy] = useState(false);

  const reload = () =>
    scheduledApi
      .list(workspaceId)
      .then(setItems)
      .catch((err: unknown) => onError(err instanceof Error ? err.message : "Could not load scheduled tasks."));

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  const selected = items.find((task) => task.id === selectedId) ?? null;

  useEffect(() => {
    setRuns([]);
    if (!selectedId) return;
    scheduledApi
      .runs(selectedId)
      .then(setRuns)
      .catch(() => setRuns([]));
  }, [selectedId, items]);

  /** Opens the form filled in with a task's current settings. */
  const startEdit = (task: ScheduledTask) => {
    setForm({
      name: task.name,
      prompt: task.prompt,
      frequency: task.frequency,
      time_of_day: task.time_of_day,
      weekday: task.weekday ?? 1,
      project_id: task.project_id ?? "",
    });
    setEditingTask(task);
  };

  const closeForm = () => {
    setCreating(false);
    setEditingTask(null);
    setForm(emptyForm);
  };

  const create = async () => {
    const body: TaskInput = {
      name: form.name.trim(),
      prompt: form.prompt.trim(),
      frequency: form.frequency,
      time_of_day: form.time_of_day,
      weekday: form.frequency === "weekly" ? form.weekday : null,
      project_id: form.project_id || null,
    };
    setBusy(true);
    try {
      if (editingTask) {
        await scheduledApi.update(editingTask.id, body);
        const id = editingTask.id;
        closeForm();
        await reload();
        setSelectedId(id);
        return;
      }
      const created = await scheduledApi.create(workspaceId, body);
      closeForm();
      setForm(emptyForm);
      await reload();
      setSelectedId(created.id);
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not create the task.");
    } finally {
      setBusy(false);
    }
  };

  const setEnabled = async (task: ScheduledTask, enabled: boolean) => {
    try {
      await scheduledApi.update(task.id, { enabled });
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not change the task.");
    }
  };

  const runNow = async (task: ScheduledTask) => {
    try {
      const result = await scheduledApi.runNow(task.id);
      await reload();
      onOpenThread(result.thread_id);
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not run the task.");
    }
  };

  const remove = async (task: ScheduledTask) => {
    try {
      await scheduledApi.remove(task.id);
      setSelectedId(null);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not delete the task.");
    }
  };

  // The new task form.
  if (creating || editingTask) {
    return (
      <main className="customize-page">
        <div className="customize-inner">
          <button className="link-button" onClick={closeForm}>← Scheduled tasks</button>
          <h1 className="projects-header">{editingTask ? "Edit scheduled task" : "New scheduled task"}</h1>
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
              <span>Instructions</span>
              <textarea
                value={form.prompt}
                onChange={(e) => setForm({ ...form, prompt: e.target.value })}
                rows={6}
                maxLength={100000}
                required
                placeholder="What should the chat do each time it runs?"
              />
            </label>
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
                type="time"
                value={form.frequency === "hourly" ? `00:${form.time_of_day.slice(3) || "00"}` : form.time_of_day}
                onChange={(e) => setForm({ ...form, time_of_day: e.target.value })}
                required
              />
            </label>
            <label className="field">
              <span>Project (optional)</span>
              <select value={form.project_id} onChange={(e) => setForm({ ...form, project_id: e.target.value })}>
                <option value="">No project</option>
                {projects.map((project) => (
                  <option key={project.id} value={project.id}>
                    {project.name}
                  </option>
                ))}
              </select>
            </label>
            <div className="form-actions">
              <button type="button" onClick={closeForm}>
                Cancel
              </button>
              <button className="primary" type="submit" disabled={busy || !form.name.trim() || !form.prompt.trim()}>
                {editingTask ? "Save changes" : "Create task"}
              </button>
            </div>
          </form>
        </div>
      </main>
    );
  }

  // The task page: next run, history, instructions and repeat.
  if (selected) {
    return (
      <main className="customize-page project-page">
        <div className="customize-inner">
          <nav className="project-crumbs">
            <button className="link-button" onClick={() => setSelectedId(null)}>Scheduled tasks</button>
            <span aria-hidden="true">/</span>
            <span>{selected.name}</span>
          </nav>
          <div className="task-heading">
            <h1 className="project-title">{selected.name}</h1>
            <div className="task-actions">
              <button className="primary" onClick={() => void runNow(selected)}>Run now</button>
              {canEdit(selected) && <button onClick={() => startEdit(selected)}>Edit</button>}
              <label className="switch-label">
                <span>{selected.enabled ? "Active" : "Paused"}</span>
                <input type="checkbox" className="switch" checked={selected.enabled} onChange={(e) => void setEnabled(selected, e.target.checked)} aria-label="Active" />
              </label>
            </div>
          </div>
          <p className="muted">
            {selected.enabled ? `Next run: ${whenLabel(selected.next_run_at)}` : "Paused. It will not run until it is turned on."}
          </p>

          <div className="project-layout">
            <section className="project-main">
              <h2 className="section-label">History</h2>
              {runs.length === 0 && <p className="muted small">No runs yet.</p>}
              <div className="recent-list">
                {runs.map((run) => (
                  <button
                    key={run.id}
                    className="recent-row"
                    disabled={!run.thread_id}
                    onClick={() => run.thread_id && onOpenThread(run.thread_id)}
                  >
                    <span>{whenLabel(run.started_at)}</span>
                    <span className="muted small">{run.title ? "Open chat" : "Chat removed"}</span>
                  </button>
                ))}
              </div>
            </section>

            <aside className="project-side">
              <div className="detail-rows">
                <div className="detail-row">
                  <div>
                    <div className="detail-label">Instructions</div>
                    <div className="muted small detail-value task-prompt">{selected.prompt}</div>
                  </div>
                </div>
                <div className="detail-row">
                  <div>
                    <div className="detail-label">Repeats</div>
                    <div className="muted small detail-value">{describeRepeat(selected)}</div>
                  </div>
                </div>
                <div className="detail-row">
                  <div>
                    <div className="detail-label">Permissions</div>
                    <div className="muted small detail-value">Asks before running commands.</div>
                  </div>
                </div>
                {canEdit(selected) && (
                  <div className="detail-actions">
                    <button onClick={() => void remove(selected)}>Delete task</button>
                  </div>
                )}
              </div>
            </aside>
          </div>
        </div>
      </main>
    );
  }

  // The list.
  const shown = items.filter((task) => {
    const q = query.trim().toLowerCase();
    return !q || task.name.toLowerCase().includes(q) || task.prompt.toLowerCase().includes(q);
  });
  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>← Back to chat</button>
        <div className="projects-header">
          <h1>Scheduled tasks</h1>
          <div className="task-toolbar">
            {showSearch && (
              <input
                className="project-search task-search"
                placeholder="Search tasks"
                aria-label="Search tasks"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                autoFocus
              />
            )}
            <button className="icon-button" aria-label="Search tasks" onClick={() => setShowSearch(!showSearch)}>
              <SearchIcon />
            </button>
            <button className="primary add-button" onClick={() => setCreating(true)}>
              <PlusIcon width={16} height={16} /> New task
            </button>
          </div>
        </div>
        {items.length === 0 && <p className="empty-state">No scheduled tasks yet.</p>}
        {items.length > 0 && shown.length === 0 && <p className="empty-state">No tasks match “{query}”.</p>}
        <div className="projects-grid">
          {shown.map((task) => (
            <button key={task.id} className="project-card task-card" onClick={() => setSelectedId(task.id)}>
              <span className="task-card-text">
                <span className="project-card-name">{task.name}</span>
                <span className="muted small task-card-prompt">{task.prompt}</span>
                <span className="muted small">
                  {task.enabled ? "Active" : "Paused"} · {describeRepeat(task)}
                </span>
              </span>
            </button>
          ))}
        </div>
      </div>
    </main>
  );
}
