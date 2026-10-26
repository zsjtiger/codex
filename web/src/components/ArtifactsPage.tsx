import { useEffect, useState } from "react";
import { artifacts as artifactApi } from "../api/resources";
import type { Artifact, ArtifactKind, Project } from "../api/types";

const kinds: { kind: ArtifactKind; label: string }[] = [
  { kind: "document", label: "Docs" },
  { kind: "slides", label: "Slides" },
  { kind: "design", label: "Design" },
  { kind: "dashboard", label: "Dashboard" },
];

interface ArtifactsPageProps {
  workspaceId: string;
  userId: string;
  projects: Project[];
  canEdit: (artifact: Artifact) => boolean;
  onBack: () => void;
  onError: (message: string) => void;
}

/**
 * Artifacts are stored as text. Every kind is saved and edited the same way; there is no
 * renderer for slides, designs or dashboards yet.
 */
export function ArtifactsPage({ workspaceId, userId, projects, canEdit, onBack, onError }: ArtifactsPageProps) {
  const [tab, setTab] = useState<"All" | "Yours">("All");
  const [items, setItems] = useState<Artifact[]>([]);
  const [open, setOpen] = useState<Artifact | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [draftContent, setDraftContent] = useState("");

  const reload = () =>
    artifactApi
      .list(workspaceId)
      .then(setItems)
      .catch((err) => onError(err instanceof Error ? err.message : "Could not load artifacts."));

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  const openArtifact = (artifact: Artifact) => {
    setOpen(artifact);
    setDraftTitle(artifact.title);
    setDraftContent(artifact.content);
  };

  const createNew = async (kind: ArtifactKind, label: string) => {
    try {
      const created = await artifactApi.create(workspaceId, { title: `Untitled ${label.toLowerCase()}`, kind });
      await reload();
      openArtifact(created);
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not create the artifact.");
    }
  };

  const save = async () => {
    if (!open) return;
    try {
      const updated = await artifactApi.update(open.id, { title: draftTitle.trim(), content: draftContent });
      setOpen(updated);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not save the artifact.");
    }
  };

  const setProject = async (projectId: string) => {
    if (!open) return;
    try {
      const updated = await artifactApi.update(open.id, { project_id: projectId || null });
      setOpen(updated);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not move the artifact.");
    }
  };

  const remove = async (artifact: Artifact) => {
    try {
      await artifactApi.remove(artifact.id);
      setOpen(null);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not delete the artifact.");
    }
  };

  if (open) {
    const editable = canEdit(open);
    return (
      <main className="artifacts-page">
        <div className="artifacts-inner">
          <button className="link-button" onClick={() => setOpen(null)}>
            ← All artifacts
          </button>
          <div className="editor">
            <input className="editor-title" value={draftTitle} onChange={(e) => setDraftTitle(e.target.value)} maxLength={200} disabled={!editable} aria-label="Title" />
            <div className="muted small">
              {open.kind} · updated {new Date(open.updated_at).toLocaleString()}
            </div>
            <textarea className="editor-body" value={draftContent} onChange={(e) => setDraftContent(e.target.value)} rows={18} maxLength={500000} disabled={!editable} aria-label="Content" />
            <div className="form-actions">
              <select value={open.project_id ?? ""} onChange={(e) => void setProject(e.target.value)} disabled={!editable} aria-label="Project">
                <option value="">No project</option>
                {projects.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
              {editable && <button onClick={() => void remove(open)}>Delete</button>}
              {editable && (
                <button className="primary" onClick={() => void save()} disabled={!draftTitle.trim()}>
                  Save
                </button>
              )}
            </div>
          </div>
        </div>
      </main>
    );
  }

  const visible = tab === "All" ? items : items.filter((a) => a.created_by === userId);

  return (
    <main className="artifacts-page">
      <div className="artifacts-inner">
        <button className="link-button" onClick={onBack}>
          ← Back to chat
        </button>
        <h1>Artifacts</h1>

        <div className="tabs" role="tablist">
          {(["All", "Yours"] as const).map((name) => (
            <button key={name} role="tab" aria-selected={tab === name} className={`tab ${tab === name ? "tab-active" : ""}`} onClick={() => setTab(name)}>
              {name}
            </button>
          ))}
        </div>

        <section>
          <h2 className="section-label">Make something new</h2>
          <div className="creator-grid">
            {kinds.map(({ kind, label }, index) => (
              <button key={kind} className={`creator tone-${["blue", "orange", "purple", "red"][index]}`} onClick={() => void createNew(kind, label)}>
                <span className="creator-icon" aria-hidden="true" />
                <span className="creator-label">{label}</span>
              </button>
            ))}
          </div>
        </section>

        <section>
          {visible.length === 0 ? (
            <p className="empty-state">{tab === "All" ? "No artifacts yet" : "Nothing created by you yet"}</p>
          ) : (
            <div className="artifact-grid">
              {visible.map((artifact) => (
                <button key={artifact.id} className="artifact-card" onClick={() => openArtifact(artifact)}>
                  <div className="artifact-preview" aria-hidden="true">
                    <div className="artifact-page" />
                  </div>
                  <div className="artifact-title">{artifact.title}</div>
                  <div className="artifact-meta">
                    {artifact.kind} · {new Date(artifact.updated_at).toLocaleDateString()}
                  </div>
                </button>
              ))}
            </div>
          )}
        </section>
      </div>
    </main>
  );
}
