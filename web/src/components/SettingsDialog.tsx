import { useEffect, useState } from "react";
import { memory as memoryApi } from "../api/resources";
import type { MemoryImportResult, MemoryNote, User, Workspace } from "../api/types";
import { AuditSection, MembersSection, WorkspaceSection } from "./WorkspaceSections";
import { useI18n } from "../i18n";

/** Asked of claude.ai or Codex so their memory can be pasted here. */
const IMPORT_PROMPT =
  "List everything you remember about me as concise bullet points, one fact per line, each starting with \"- \". " +
  "Include my preferences, my work and projects, and recurring context. Output only the list.";

interface SettingsDialogProps {
  user: User;
  workspace: Workspace | null;
  onClose: () => void;
  onError: (message: string) => void;
}

type Section = "account" | "memory" | "workspace" | "members" | "audit";

/** Account settings. Memory holds notes that are added to the user's new chats when it is on. */
export function SettingsDialog({ user, workspace, onClose, onError }: SettingsDialogProps) {
  const [section, setSection] = useState<Section>("memory");
  const { t, locale, setLocale } = useI18n();
  // Workspace administration is for admins and owners only.
  const isAdmin = workspace?.role === "owner" || workspace?.role === "admin";
  const [enabled, setEnabled] = useState(true);
  const [notes, setNotes] = useState<MemoryNote[] | null>(null);
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState<{ id: string; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [pasted, setPasted] = useState("");
  const [importResult, setImportResult] = useState<MemoryImportResult | null>(null);

  useEffect(() => {
    if (section !== "memory") return;
    memoryApi
      .get()
      .then((view) => {
        setEnabled(view.enabled);
        setNotes(view.notes);
      })
      .catch((err: unknown) => onError(err instanceof Error ? err.message : "Could not load memory."));
  }, [section, onError]);

  const reload = () =>
    memoryApi
      .get()
      .then((view) => setNotes(view.notes))
      .catch((err: unknown) => onError(err instanceof Error ? err.message : "Could not load memory."));

  const toggle = async (next: boolean) => {
    setEnabled(next);
    try {
      await memoryApi.setEnabled(next);
    } catch (err) {
      setEnabled(!next);
      onError(err instanceof Error ? err.message : "Could not change memory.");
    }
  };

  const add = async () => {
    if (!draft.trim()) return;
    setBusy(true);
    try {
      await memoryApi.add(draft.trim());
      setDraft("");
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not add the note.");
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    if (!editing || !editing.text.trim()) return;
    setBusy(true);
    try {
      await memoryApi.edit(editing.id, editing.text.trim());
      setEditing(null);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not save the note.");
    } finally {
      setBusy(false);
    }
  };

  const remove = async (note: MemoryNote) => {
    try {
      await memoryApi.remove(note.id);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not delete the note.");
    }
  };

  const runImport = async (input: string | File) => {
    setBusy(true);
    setImportResult(null);
    try {
      const result = await memoryApi.importNotes(input);
      setImportResult(result);
      setPasted("");
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not import the memory.");
    } finally {
      setBusy(false);
    }
  };

  const copyPrompt = async () => {
    try {
      await navigator.clipboard.writeText(IMPORT_PROMPT);
    } catch {
      // Clipboard access can be refused; the prompt stays selectable.
    }
  };

  const exportMarkdown = () => {
    const body = (notes ?? []).map((note) => `- ${note.content.replace(/\n/g, " ")}`).join("\n");
    const blob = new Blob([`# Memory\n\n${body}\n`], { type: "text/markdown" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "memory.md";
    link.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div className="settings-dialog" role="dialog" aria-modal="true" aria-label="Settings" onMouseDown={(event) => event.stopPropagation()}>
        <header className="settings-header">
          <h2>{t("settings.title")}</h2>
          <button className="icon-button" onClick={onClose} aria-label={t("settings.close")}>×</button>
        </header>
        <div className="settings-body">
          <nav className="settings-nav" aria-label="Settings sections">
            <button className={section === "account" ? "settings-nav-item settings-nav-active" : "settings-nav-item"} onClick={() => setSection("account")}>{t("settings.account")}</button>
            <button className={section === "memory" ? "settings-nav-item settings-nav-active" : "settings-nav-item"} onClick={() => setSection("memory")}>{t("settings.memory")}</button>
            {isAdmin && workspace && (
              <>
                <div className="settings-nav-heading">{t("settings.workspace")}</div>
                <button className={section === "workspace" ? "settings-nav-item settings-nav-active" : "settings-nav-item"} onClick={() => setSection("workspace")}>{t("settings.title")}</button>
                <button className={section === "members" ? "settings-nav-item settings-nav-active" : "settings-nav-item"} onClick={() => setSection("members")}>{t("settings.members")}</button>
                <button className={section === "audit" ? "settings-nav-item settings-nav-active" : "settings-nav-item"} onClick={() => setSection("audit")}>{t("settings.auditLog")}</button>
              </>
            )}
          </nav>

          <div className="settings-content">
            {section === "account" && (
              <>
                <h3 className="settings-title">{t("settings.account")}</h3>
                <div className="settings-row">
                  <div>
                    <div className="detail-label">{t("settings.name")}</div>
                    <div className="muted small">{user.display_name || "Not set"}</div>
                  </div>
                </div>
                <div className="settings-row">
                  <div>
                    <div className="detail-label">{t("settings.email")}</div>
                    <div className="muted small">{user.email}</div>
                  </div>
                </div>
                <div className="settings-row">
                  <div>
                    <div className="detail-label">{t("settings.language")}</div>
                    <div className="muted small">{t("settings.languageHint")}</div>
                  </div>
                  <select value={locale} onChange={(e) => setLocale(e.target.value as "en" | "zh")} aria-label={t("settings.language")}>
                    <option value="en">English</option>
                    <option value="zh">中文</option>
                  </select>
                </div>
              </>
            )}

            {section === "workspace" && isAdmin && workspace && <WorkspaceSection workspace={workspace} onError={onError} />}
            {section === "members" && isAdmin && workspace && <MembersSection workspace={workspace} user={user} onError={onError} />}
            {section === "audit" && isAdmin && workspace && <AuditSection workspace={workspace} onError={onError} />}

            {section === "memory" && (
              <>
                <h3 className="settings-title">Memory</h3>
                <div className="settings-row">
                  <div>
                    <div className="detail-label">Use saved memory in chats</div>
                    <div className="muted small">Your notes below are added to new chats when this is on.</div>
                  </div>
                  <input type="checkbox" className="switch" checked={enabled} onChange={(e) => void toggle(e.target.checked)} aria-label="Use saved memory in chats" />
                </div>

                <div className="settings-row settings-row-top">
                  <div className="settings-grow">
                    <div className="detail-label">Your memory</div>
                    <div className="muted small">Things Claude should remember about you and your work. Up to 200 notes.</div>
                    <div className="memory-list">
                      {notes && notes.length === 0 && <p className="muted small">No notes yet.</p>}
                      {notes?.map((note) =>
                        editing?.id === note.id ? (
                          <div key={note.id} className="memory-edit">
                            <textarea value={editing.text} onChange={(e) => setEditing({ id: note.id, text: e.target.value })} rows={3} maxLength={2000} aria-label="Edit note" />
                            <div className="form-actions">
                              <button onClick={() => setEditing(null)}>Cancel</button>
                              <button className="primary" disabled={busy || !editing.text.trim()} onClick={() => void save()}>Save</button>
                            </div>
                          </div>
                        ) : (
                          <div key={note.id} className="memory-note">
                            <span className="memory-note-text">{note.content}</span>
                            <button onClick={() => setEditing({ id: note.id, text: note.content })}>Edit</button>
                            <button onClick={() => void remove(note)}>Delete</button>
                          </div>
                        ),
                      )}
                    </div>
                    <form
                      className="memory-add"
                      onSubmit={(event) => {
                        event.preventDefault();
                        void add();
                      }}
                    >
                      <input value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Add a note, e.g. I prefer short answers" maxLength={2000} aria-label="New memory note" />
                      <button className="primary" type="submit" disabled={busy || !draft.trim()}>Add</button>
                    </form>
                  </div>
                </div>

                <div className="settings-row settings-row-top">
                  <div className="settings-grow">
                    <div className="detail-label">Import memory from other AI providers</div>
                    <div className="muted small">
                      Bring notes from claude.ai or Codex. Duplicates are skipped.
                    </div>
                    {!importOpen && (
                      <div>
                        <button onClick={() => setImportOpen(true)}>Start import</button>
                      </div>
                    )}
                    {importOpen && (
                      <div className="import-panel">
                        <div className="muted small">1. Copy this prompt and send it to claude.ai or Codex:</div>
                        <div className="import-prompt">
                          <code>{IMPORT_PROMPT}</code>
                          <button onClick={() => void copyPrompt()}>Copy</button>
                        </div>
                        <div className="muted small">2. Paste the answer here, or upload a file:</div>
                        <textarea
                          value={pasted}
                          onChange={(e) => setPasted(e.target.value)}
                          rows={5}
                          placeholder="- Prefers short answers&#10;- Works on a robotics team"
                          aria-label="Pasted memory"
                        />
                        <div className="import-actions">
                          <button className="primary" disabled={busy || !pasted.trim()} onClick={() => void runImport(pasted)}>
                            Import text
                          </button>
                          <label className="import-file">
                            Upload file (.zip, .md or .txt)
                            <input
                              type="file"
                              accept=".zip,.md,.markdown,.txt"
                              hidden
                              disabled={busy}
                              onChange={(e) => {
                                const file = e.target.files?.[0];
                                e.target.value = "";
                                if (file) void runImport(file);
                              }}
                            />
                          </label>
                          <span className="muted small">Codex keeps memory in MEMORY.md under its memories folder.</span>
                        </div>
                        {importResult && (
                          <p className="banner small">
                            Added {importResult.added} notes. {importResult.duplicates} were already saved
                            {importResult.skipped > 0 ? `; ${importResult.skipped} left out because memory is full` : ""}.
                          </p>
                        )}
                      </div>
                    )}
                  </div>
                </div>

                <div className="settings-row">
                  <div>
                    <div className="detail-label">Export memory</div>
                    <div className="muted small">Download your notes as a Markdown file.</div>
                  </div>
                  <button onClick={exportMarkdown} disabled={!notes || notes.length === 0}>Export</button>
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
