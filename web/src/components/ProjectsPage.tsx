import { useEffect, useState } from "react";
import { attachments as attachmentApi, contentTypeOf, projects as projectApi } from "../api/resources";
import type { Attachment, Project } from "../api/types";
import { PlusIcon } from "./icons";

interface ProjectsPageProps {
  workspaceId: string;
  projects: Project[];
  reload: () => Promise<void>;
  canEdit: (project: Project) => boolean;
  onStartChat: (projectId: string) => void;
  onBack: () => void;
  onError: (message: string) => void;
}

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

  return (
    <div className="project-files">
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
      {files && files.length === 0 && <p className="muted small">No files yet. Text files (txt, md, csv, json) are read into new chats.</p>}
      {files?.map((file) => (
        <div key={file.id} className="project-file-row">
          <span className="project-file-name">{file.filename}</span>
          <span className="muted small">
            {file.content_type.startsWith("text/") || file.content_type === "application/json" ? "read into chats" : "stored only"}
          </span>
          <button onClick={() => void remove(file)}>Delete</button>
        </div>
      ))}
    </div>
  );
}

/** Projects group chats and carry instructions that apply to every thread in them. */
export function ProjectsPage({ workspaceId, projects, reload, canEdit, onStartChat, onBack, onError }: ProjectsPageProps) {
  const [editing, setEditing] = useState<Project | "new" | null>(null);
  const [name, setName] = useState("");
  const [instructions, setInstructions] = useState("");
  const [busy, setBusy] = useState(false);
  const [filesOpen, setFilesOpen] = useState<string | null>(null);

  const open = (project: Project | "new") => {
    setEditing(project);
    setName(project === "new" ? "" : project.name);
    setInstructions(project === "new" ? "" : project.instructions);
  };

  const save = async () => {
    if (!editing || !name.trim()) return;
    setBusy(true);
    try {
      if (editing === "new") await projectApi.create(workspaceId, { name: name.trim(), instructions });
      else await projectApi.update(editing.id, { name: name.trim(), instructions });
      setEditing(null);
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
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not delete the project.");
    }
  };

  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>
          ← Back to chat
        </button>
        <h1>Projects</h1>

        {editing ? (
          <form
            className="form-card"
            onSubmit={(event) => {
              event.preventDefault();
              void save();
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
              <button type="button" onClick={() => setEditing(null)}>
                Cancel
              </button>
              <button className="primary" type="submit" disabled={busy || !name.trim()}>
                Save
              </button>
            </div>
          </form>
        ) : (
          <>
            <div className="customize-toolbar">
              <span />
              <button className="add-button" onClick={() => open("new")}>
                <PlusIcon width={16} height={16} /> New project
              </button>
            </div>
            {projects.length === 0 && <p className="empty-state">No projects yet.</p>}
            <div className="list">
              {projects.map((project) => (
                <div key={project.id} className="list-row">
                  <div className="list-main">
                    <div className="list-title">{project.name}</div>
                    <div className="list-sub">{project.instructions || "No instructions"}</div>
                  </div>
                  <div className="list-actions">
                    <button onClick={() => setFilesOpen(filesOpen === project.id ? null : project.id)} aria-expanded={filesOpen === project.id}>
                      Files
                    </button>
                    <button onClick={() => onStartChat(project.id)}>Start chat</button>
                    {canEdit(project) && <button onClick={() => open(project)}>Edit</button>}
                    {canEdit(project) && <button onClick={() => void remove(project)}>Delete</button>}
                  </div>
                  {filesOpen === project.id && <ProjectFiles workspaceId={workspaceId} projectId={project.id} onError={onError} />}
                </div>
              ))}
            </div>
          </>
        )}
      </div>
    </main>
  );
}
