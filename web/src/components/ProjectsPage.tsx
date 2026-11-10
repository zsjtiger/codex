import { useEffect, useState } from "react";
import { attachments as attachmentApi, contentTypeOf, projects as projectApi } from "../api/resources";
import type { Attachment, Project, Thread } from "../api/types";
import { PlusIcon } from "./icons";

interface ProjectsPageProps {
  workspaceId: string;
  projects: Project[];
  threads: Thread[];
  reload: () => Promise<void>;
  canEdit: (project: Project) => boolean;
  onStartChat: (projectId: string) => void;
  onOpenThread: (threadId: string) => void;
  onBack: () => void;
  onError: (message: string) => void;
}

/** Bytes of project file text a chat reads per turn. Matches the gateway's limit. */
const PROJECT_READ_LIMIT_BYTES = 60000;

/** Files kept with a project. Text files are read into every new chat in the project. */
function ProjectFiles({ workspaceId, projectId, onError }: { workspaceId: string; projectId: string; onError: (message: string) => void }) {
  const [files, setFiles] = useState<Attachment[] | null>(null);
  const [uploading, setUploading] = useState(false);

  const load = () =>
    attachmentApi
      .listForProject(workspaceId, projectId)
      .then(setFiles)
      .catch((err: unknown) => onError(err instanceof Error ? err.message : "Could not load the files."));

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId, projectId]);

  const upload = async (file: File) => {
    setUploading(true);
    try {
      const created = await attachmentApi.create(workspaceId, {
        filename: file.name,
        content_type: contentTypeOf(file),
        size_bytes: file.size,
        project_id: projectId,
      });
      await attachmentApi.upload(created.id, file);
      await load();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not upload the file.");
    } finally {
      setUploading(false);
    }
  };

  const remove = async (file: Attachment) => {
    try {
      await attachmentApi.remove(file.id);
      await load();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not delete the file.");
    }
  };

  // Chats read up to this many characters of a project's files in total.
  const readTotal = (files ?? []).reduce((sum, file) => sum + (file.read_bytes ?? 0), 0);
  const overLimit = readTotal > PROJECT_READ_LIMIT_BYTES;

  return (
    <div className="project-files">
      {overLimit && (
        <p className="banner error small">
          Chats read only the first {PROJECT_READ_LIMIT_BYTES.toLocaleString()} bytes of these files ({readTotal.toLocaleString()} bytes in total). The rest is not included.
        </p>
      )}
      <div className="project-files-header">
        <span className="muted small">Files</span>
        <label className="add-button">
          {uploading ? "Uploading…" : "Upload file"}
          <input
            type="file"
            hidden
            disabled={uploading}
            onChange={(event) => {
              const file = event.target.files?.[0];
              event.target.value = "";
              if (file) void upload(file);
            }}
          />
        </label>
      </div>
      {files && files.length === 0 && <p className="muted small">No files yet. Text, PDF and Word files are read into new chats.</p>}
      {files?.map((file) => (
        <div key={file.id} className="project-file-row">
          <span className="project-file-name">{file.filename}</span>
          <span className="muted small">
            {file.read_bytes ? `read ${file.read_bytes.toLocaleString()} bytes` : "stored only"}
          </span>
          <button onClick={() => void remove(file)}>Delete</button>
        </div>
      ))}
    </div>
  );
}

function shortDate(value: string): string {
  return new Date(value).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function FolderIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    </svg>
  );
}

/** Name and instructions form, for a new project or an existing one. */
function ProjectForm({ initial, onSave, onCancel, busy }: { initial: Project | null; onSave: (name: string, instructions: string) => void; onCancel: () => void; busy: boolean }) {
  const [name, setName] = useState(initial?.name ?? "");
  const [instructions, setInstructions] = useState(initial?.instructions ?? "");
  return (
    <form
      className="form-card"
      onSubmit={(event) => {
        event.preventDefault();
        if (name.trim()) onSave(name.trim(), instructions);
      }}
    >
      <label className="field">
        <span>Name</span>
        <input value={name} onChange={(e) => setName(e.target.value)} maxLength={100} required />
      </label>
      <label className="field">
        <span>Instructions</span>
        <textarea
          value={instructions}
          onChange={(e) => setInstructions(e.target.value)}
          rows={6}
          maxLength={20000}
          placeholder="Added to every new chat in this project."
        />
      </label>
      <div className="form-actions">
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
        <button className="primary" type="submit" disabled={busy || !name.trim()}>
          Save
        </button>
      </div>
    </form>
  );
}

/** Projects group chats, files and instructions. The list shows cards; a card opens the project. */
export function ProjectsPage({ workspaceId, projects, threads, reload, canEdit, onStartChat, onOpenThread, onBack, onError }: ProjectsPageProps) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [editing, setEditing] = useState(false);
  const [showFiles, setShowFiles] = useState(false);
  const [fileCount, setFileCount] = useState<number | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);

  const selected = projects.find((p) => p.id === selectedId) ?? null;
  const projectThreads = selected ? threads.filter((t) => t.project_id === selected.id && !t.archived_at) : [];

  useEffect(() => {
    setShowFiles(false);
    setEditing(false);
    setFileCount(null);
    if (!selected) return;
    attachmentApi
      .listForProject(workspaceId, selected.id)
      .then((files: Attachment[]) => setFileCount(files.length))
      .catch(() => setFileCount(null));
    // Reloads when the selected project changes, not on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId, workspaceId]);

  const save = async (name: string, instructions: string) => {
    setBusy(true);
    try {
      if (creating) {
        const created = await projectApi.create(workspaceId, { name, instructions });
        setCreating(false);
        setSelectedId(created.id);
      } else if (selected) {
        await projectApi.update(selected.id, { name, instructions });
        setEditing(false);
      }
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not save the project.");
    } finally {
      setBusy(false);
    }
  };

  const remove = async (project: Project) => {
    try {
      await projectApi.remove(project.id);
      setSelectedId(null);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not delete the project.");
    }
  };

  // The project page.
  if (selected) {
    return (
      <main className="customize-page project-page">
        <div className="customize-inner">
          <nav className="project-crumbs">
            <button className="link-button" onClick={() => setSelectedId(null)}>Projects</button>
            <span aria-hidden="true">/</span>
            <span>{selected.name}</span>
          </nav>
          <h1 className="project-title">{selected.name}</h1>

          {message && <p className="banner error" role="alert">{message}</p>}

          <div className="project-layout">
            <section className="project-main">
              <div className="new-session">
                <input
                  value=""
                  readOnly
                  placeholder={`New session in ${selected.name}`}
                  onFocus={() => onStartChat(selected.id)}
                  aria-label="Start a new session in this project"
                />
                <button className="primary" onClick={() => onStartChat(selected.id)}>Start chat</button>
              </div>

              <h2 className="section-label">Recents</h2>
              {projectThreads.length === 0 && <p className="muted small">No chats in this project yet.</p>}
              <div className="recent-list">
                {projectThreads.map((thread) => (
                  <button key={thread.id} className="recent-row" onClick={() => onOpenThread(thread.id)}>
                    <span>{thread.title}</span>
                    <span className="muted small">{shortDate(thread.last_activity_at)}</span>
                  </button>
                ))}
              </div>
            </section>

            <aside className="project-side">
              {editing ? (
                <ProjectForm
                  initial={selected}
                  busy={busy}
                  onSave={(name, instructions) => void save(name, instructions)}
                  onCancel={() => setEditing(false)}
                />
              ) : (
                <div className="detail-rows">
                  <div className="detail-row">
                    <div>
                      <div className="detail-label">Instructions</div>
                      <div className="muted small detail-value">{selected.instructions || "Not set"}</div>
                    </div>
                    {canEdit(selected) && <button className="link-button" onClick={() => setEditing(true)}>{selected.instructions ? "Edit" : "Add"}</button>}
                  </div>
                  <div className="detail-row">
                    <div>
                      <div className="detail-label">Context</div>
                      <div className="muted small detail-value">{fileCount === null ? "" : fileCount === 0 ? "No files" : `${fileCount} file${fileCount === 1 ? "" : "s"}`}</div>
                    </div>
                    <button className="link-button" onClick={() => setShowFiles((open) => !open)} aria-expanded={showFiles}>
                      {showFiles ? "Hide" : "Add"}
                    </button>
                  </div>
                  {showFiles && (
                    <ProjectFiles
                      workspaceId={workspaceId}
                      projectId={selected.id}
                      onError={(text) => {
                        setMessage(text);
                        onError(text);
                      }}
                    />
                  )}
                  {canEdit(selected) && (
                    <div className="detail-actions">
                      <button onClick={() => void remove(selected)}>Delete project</button>
                    </div>
                  )}
                </div>
              )}
            </aside>
          </div>
        </div>
      </main>
    );
  }

  // The list of projects, as cards.
  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>← Back to chat</button>
        <div className="projects-header">
          <h1>Projects</h1>
          {!creating && (
            <button className="primary add-button" onClick={() => setCreating(true)}>
              <PlusIcon width={16} height={16} /> New project
            </button>
          )}
        </div>

        {creating ? (
          <ProjectForm initial={null} busy={busy} onSave={(name, instructions) => void save(name, instructions)} onCancel={() => setCreating(false)} />
        ) : (
          <>
            {projects.length === 0 && <p className="empty-state">No projects yet.</p>}
            <div className="projects-grid">
              {projects.map((project) => (
                <button key={project.id} className="project-card" onClick={() => setSelectedId(project.id)}>
                  <span className="project-card-icon"><FolderIcon /></span>
                  <span className="project-card-text">
                    <span className="project-card-name">{project.name}</span>
                    <span className="muted small">{shortDate(project.updated_at)}</span>
                  </span>
                </button>
              ))}
            </div>
          </>
        )}
      </div>
    </main>
  );
}
