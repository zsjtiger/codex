import { useCallback, useEffect, useState } from "react";
import { artifacts as artifactApi, skills as skillApi, threads as threadApi } from "../api/resources";
import type { Artifact, Environment, FileMatch, Project, Skill, Thread, Turn, User } from "../api/types";
import type { Activity, Approval, SendOptions } from "../state/useCodex";
import type { ToolUse } from "../state/toolUses";
import type { WorkLog } from "../state/work";
import type { CommandOutput } from "../state/commandOutput";
import { CheckoutFiles } from "./CheckoutFiles";
import { RunCommand } from "./RunCommand";
import { WorkspaceChanges } from "./WorkspaceChanges";
import { BackgroundTerminals } from "./BackgroundTerminals";
import { GoalBar } from "./GoalBar";
import type { Notice } from "../state/notices";
import type { Prediction } from "../state/useCodex";
import { Composer } from "./Composer";
import { usePreview } from "./Preview";
import { Conversation } from "./Conversation";
import { liveDocumentEdit } from "./documentEdits";
import type { PluginDraft } from "../state/pluginDraft";
import { ChevronIcon, FileIcon, SparkIcon } from "./icons";
import { useI18n } from "../i18n";
import { tx } from "../i18n";

interface HomeProps {
  /** Text the composer starts with, from the plugins page. */
  prefill?: string | null;
  onPrefillUsed?: () => void;
  /** Starts a new chat from a reply of this one. */
  onFork?: (turnId: string) => void;
  onRevert?: (turnId: string) => void;
  predictions: Record<string, Prediction>;
  onSuggestNext: (turn: Turn) => void;
  onDismissSuggestion: (codexTurnId: string) => void;
  /** A review or a compaction of the open chat. */
  onAction?: (action: "review" | "compact") => void;
  /** Saves a plugin a reply designed into the workspace. */
  onAddPlugin?: (draft: PluginDraft) => Promise<void>;
  user: User;
  workspaceId: string;
  thread: Thread | null;
  threads: Thread[];
  onOpenThread: (threadId: string) => void;
  turns: Turn[];
  agentText: Record<string, string>;
  /** The assistant message each running codex turn is writing now, as it streams. */
  streamText: Record<string, string>;
  toolUses: Record<string, ToolUse[]>;
  environment: Environment | null;
  activity: Record<string, Activity>;
  work: Record<string, WorkLog>;
  diffs: Record<string, string>;
  commandOutput: Record<string, CommandOutput>;
  approvals: Approval[];
  sending: boolean;
  projects: Project[];
  draftProjectId: string | null;
  onDraftProject: (projectId: string | null) => void;
  onCreateProject: (name: string) => Promise<string>;
  onThreadChanged: () => void;
  onSend: (text: string, options: SendOptions) => void;
  onStop: (turnId: string) => void;
  onAnswer: (approval: Approval, result: unknown) => void;
  onFeedback: (turnId: string, rating: "up" | "down" | null) => void;
  onOpenArtifacts: () => void;
  upload: (file: File) => Promise<string>;
  error: string | null;
  onError: (message: string) => void;
  onDismissError: () => void;
  notices: Notice[];
  onDismissNotice: (id: number) => void;
}

export function Home(props: HomeProps) {
  const { t } = useI18n();
  const { prefill, onPrefillUsed, onAddPlugin, onAction, onFork, onRevert, predictions, onSuggestNext, onDismissSuggestion, user, workspaceId, thread, threads, onOpenThread, turns, agentText, streamText, toolUses, activity, work, diffs, commandOutput, environment, approvals, sending, projects, draftProjectId, onDraftProject, onCreateProject, onThreadChanged, onSend, onStop, onAnswer, onFeedback, onOpenArtifacts, upload, error, onError, onDismissError, notices, onDismissNotice } = props;
  const pending = approvals.filter((a) => a.threadId === thread?.id);
  // The newest finished turn whose suggestion is ready is offered above the composer.
  const suggestedTurn = [...turns].reverse().find((turn) => turn.status === "completed" && turn.codex_turn_id && predictions[turn.codex_turn_id]?.status === "ready");
  const suggestedPrediction = suggestedTurn?.codex_turn_id ? predictions[suggestedTurn.codex_turn_id] : undefined;
  const suggestion =
    suggestedTurn?.codex_turn_id && suggestedPrediction?.status === "ready"
      ? { text: suggestedPrediction.text, onDismiss: () => onDismissSuggestion(suggestedTurn.codex_turn_id as string) }
      : null;
  const threadId = thread?.id ?? "";
  const hasRepo = Boolean(thread?.repo_id);
  const searchFiles = useCallback((query: string): Promise<FileMatch[]> => threadApi.files(threadId, query), [threadId]);
  const runningTurn = [...turns].reverse().find((t) => (t.status === "running" || t.status === "queued") && t.codex_turn_id);
  const [recent, setRecent] = useState<Artifact[]>([]);
  // Documents saved from this chat, shown as cards under the conversation.
  const [docs, setDocs] = useState<Artifact[]>([]);
  const { open, current } = usePreview();
  const [skills, setSkills] = useState<Skill[]>([]);

  useEffect(() => {
    skillApi
      .list(workspaceId, "skill")
      .then((list) => setSkills(list.filter((skill) => skill.enabled)))
      .catch(() => setSkills([]));
  }, [workspaceId]);
  const [showUsed, setShowUsed] = useState(false);
  const used = thread ? (toolUses[thread.id] ?? []) : [];
  const showUsedPanel = showUsed && used.length > 0;

  // A summary is a turn of its own, so the cards are read again as turns finish.
  const finishedTurns = turns.filter((turn) => turn.status === "completed").length;
  useEffect(() => {
    if (!thread) {
      setDocs([]);
      return;
    }
    artifactApi
      .list(workspaceId, `?kind=document&thread_id=${thread.id}`)
      .then(setDocs)
      .catch(() => setDocs([]));
  }, [workspaceId, thread?.id, finishedTurns]);

  // The model's change to a document is read from the reply as it streams. When that document is open,
  // its panel shows the change as it is written, and stops marking it as updating once the change closes.
  const liveText = runningTurn?.codex_turn_id
    ? [agentText[runningTurn.codex_turn_id], streamText[runningTurn.codex_turn_id]].filter(Boolean).join("\n\n")
    : "";
  const liveEdit = liveText ? liveDocumentEdit(liveText) : null;
  const liveId = liveEdit?.id ?? null;
  const liveContent = liveEdit?.content ?? null;
  const liveClosed = liveEdit?.closed ?? false;
  useEffect(() => {
    if (!current || current.kind !== "document" || !current.artifactId) return;
    if (liveId !== null && liveContent !== null && current.artifactId === liveId) {
      if (current.content !== liveContent || current.live !== !liveClosed) {
        open({ ...current, content: liveContent, live: !liveClosed });
      }
    } else if (current.live) {
      open({ ...current, live: false });
    }
  }, [liveId, liveContent, liveClosed, current, open]);

  useEffect(() => {
    if (thread) return;
    artifactApi
      .list(workspaceId, "?limit=4")
      .then(setRecent)
      .catch(() => setRecent([]));
  }, [workspaceId, thread]);

  const openDoc = (doc: Artifact) =>
    open({
      kind: "document",
      title: doc.title,
      content: doc.content,
      artifactId: doc.id,
      onSaved: (saved) => setDocs((prev) => prev.map((d) => (d.id === doc.id ? { ...d, ...saved } : d))),
    });

  // A reply saved as a document: its title is its first heading, or its first line.
  const saveDocument = async (reply: string) => {
    if (!thread) return;
    try {
      const doc = await artifactApi.create(workspaceId, {
        title: documentTitle(reply),
        kind: "document",
        content: reply,
        thread_id: thread.id,
      });
      setDocs((prev) => [doc, ...prev]);
      openDoc(doc);
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("Could not save the document."));
    }
  };

  // Asks the model to write a reply as a document of this chat; the card appears when that turn finishes.
  const summarize = async (turnId: string) => {
    if (!thread) return;
    try {
      await threadApi.summarize(thread.id, turnId);
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("Could not summarize the reply."));
    }
  };

  const [savedNote, setSavedNote] = useState<string | null>(null);
  const saveToProject = async (projectId: string) => {
    if (!thread || !projectId) return;
    try {
      const file = await threadApi.saveToProject(thread.id, projectId);
      const projectName = projects.find((p) => p.id === projectId)?.name ?? "project";
      setSavedNote(`${tx("Saved")} “${file.filename}” ${tx("to")} ${projectName}`);
      window.setTimeout(() => setSavedNote(null), 4000);
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("Could not save the chat to the project."));
    }
  };

  const setThreadProject = async (projectId: string | null) => {
    if (!thread) return;
    try {
      await threadApi.setProject(thread.id, projectId);
      onThreadChanged();
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("Could not change the project."));
    }
  };

  return (
    <main className={["home", thread ? "home-thread" : "", showUsedPanel ? "home-with-panel" : ""].filter(Boolean).join(" ")}>
      {error && (
        <p className="banner error" role="alert">
          {error}
          <button className="banner-close" onClick={onDismissError} aria-label={tx("Dismiss")}>
            ×
          </button>
        </p>
      )}
      {notices.map((notice) => (
        <p key={notice.id} className={notice.tone === "warning" ? "banner notice-warning" : "banner notice-info"} role="status">
          {notice.text}
          <button className="banner-close" onClick={() => onDismissNotice(notice.id)} aria-label={tx("Dismiss")}>
            ×
          </button>
        </p>
      ))}

      {showUsedPanel && <UsedPanel uses={used} onClose={() => setShowUsed(false)} />}

      {thread ? (
        <div className="thread-view">
          <header className="thread-title">
            <span>{thread.title}</span>
            {projects.length > 0 && (
              <select
                className="save-to-project"
                aria-label={tx("Save chat to project")}
                value=""
                onChange={(event) => void saveToProject(event.target.value)}
              >
                <option value="">{tx("Save to project…")}</option>
                {projects.map((project) => (
                  <option key={project.id} value={project.id}>
                    {project.name}
                  </option>
                ))}
              </select>
            )}
            {savedNote && <span className="muted small">{savedNote}</span>}
            {used.length > 0 && (
              <button
                className="used-toggle"
                aria-pressed={showUsed}
                onClick={() => setShowUsed((open) => !open)}
              >
                Used · {used.length}
              </button>
            )}
          </header>
          {thread.repo_id && <CommitBar threadId={thread.id} branch={thread.work_branch} onError={onError} />}
          {thread.repo_id && <PreviewBar threadId={thread.id} onError={onError} />}
          {thread.repo_id && <CheckoutFiles threadId={thread.id} onError={onError} />}
          {thread.repo_id && <RunCommand threadId={thread.id} output={commandOutput} onError={onError} />}
          {thread.repo_id && <WorkspaceChanges threadId={thread.id} onError={onError} />}
          {thread.repo_id && <BackgroundTerminals threadId={thread.id} onError={onError} />}
          <GoalBar threadId={thread.id} onError={onError} />
          <div className="thread-scroll">
            <Conversation
              variant={thread.repo_id ? "code" : "chat"}
              turns={turns}
              agentText={agentText}
              activity={activity}
              work={work}
              diffs={diffs}
              approvals={pending}
              onAnswer={onAnswer}
              onFeedback={onFeedback}
              onRegenerate={(input) => onSend(input, {})}
              onSaveDocument={(reply) => void saveDocument(reply)}
              onSummarize={(turnId) => void summarize(turnId)}
              onFork={onFork}
              onRevert={onRevert}
              predictions={predictions}
              onSuggest={onSuggestNext}
              onAddPlugin={onAddPlugin}
            />
            {docs.map((doc) => (
              <button key={doc.id} className="doc-card" onClick={() => openDoc(doc)}>
                <span className="doc-card-icon">
                  <FileIcon />
                </span>
                <span className="doc-card-text">
                  <strong>{doc.title}</strong>
                  <span className="muted small">{tx("Document")} · {tx("Only you")}</span>
                </span>
              </button>
            ))}
          </div>
          <div className="composer-dock">
            <Composer
              prefill={prefill}
              onPrefillUsed={onPrefillUsed}
              suggestion={suggestion}
              environmentId={environment?.id ?? null}
              onSend={onSend}
              onAction={onAction}
              onSearchFiles={hasRepo ? searchFiles : undefined}
              onStop={runningTurn ? () => onStop(runningTurn.id) : undefined}
              running={Boolean(runningTurn)}
              disabled={sending}
              placeholder={tx("Reply…")}
              projects={projects}
          skills={skills}
              projectId={thread.project_id}
              onProjectChange={(id) => void setThreadProject(id)}
              onCreateProject={onCreateProject}
              upload={upload}
              onError={onError}
            />
          </div>
        </div>
      ) : (
        <div className="home-inner">
          <h1 className="greeting">
            <SparkIcon className="greeting-mark" />
            <span>{tx("You’re here!")}</span>
          </h1>
          <Composer
            prefill={prefill}
            onPrefillUsed={onPrefillUsed}
            suggestion={suggestion}
            environmentId={environment?.id ?? null}
            onSend={onSend}
            disabled={sending}
            projects={projects}
          skills={skills}
            projectId={draftProjectId}
            onProjectChange={onDraftProject}
            onCreateProject={onCreateProject}
            upload={upload}
            onError={onError}
          />

          <ActiveList threads={threads} onOpen={onOpenThread} />

          <section className="recent-artifacts">
            <button className="recent-heading link-button" onClick={onOpenArtifacts}>
              <span>{t("home.recentArtifacts")}</span>
              <ChevronIcon />
            </button>
            {recent.length === 0 && <p className="empty-note">{t("home.noArtifacts")}</p>}
            {recent.length > 0 && (
              <div className="artifact-grid">
                {recent.map((artifact) => (
                  <button
                    key={artifact.id}
                    className="artifact-card"
                    onClick={() => open({ kind: "document", title: artifact.title, content: artifact.content })}
                  >
                    <div className="artifact-preview" aria-hidden="true">
                      <div className="artifact-page" />
                    </div>
                    <div className="artifact-title">{artifact.title}</div>
                    <div className="artifact-meta">{artifact.kind}</div>
                  </button>
                ))}
              </div>
            )}
          </section>
        </div>
      )}
    </main>
  );
}

/** A title from a reply: its first heading, else its first line, cut to a short length. */
function documentTitle(reply: string): string {
  const lines = reply.split("\n").map((line) => line.trim()).filter(Boolean);
  const heading = lines.find((line) => /^#{1,6}\s/.test(line));
  const first = (heading ?? lines[0] ?? "").replace(/^#{1,6}\s*/, "").replace(/[*_`]/g, "").trim();
  if (!first) return tx("Document");
  return first.length > 60 ? `${first.slice(0, 57)}…` : first;
}

function UsedPanel({ uses, onClose }: { uses: ToolUse[]; onClose: () => void }) {
  const groups = new Map<string, ToolUse[]>();
  for (const use of uses) groups.set(use.kind, [...(groups.get(use.kind) ?? []), use]);

  return (
    <aside className="used-panel" aria-label={tx("Used in this session")}>
      <header className="used-panel-header">
        <h2>{tx("Used in this session")}</h2>
        <button className="banner-close" onClick={onClose} aria-label={tx("Close")}>
          ×
        </button>
      </header>
      {uses.length === 0 ? (
        <p className="muted">{tx("Web searches, connectors, commands and file changes appear here as Codex uses them.")}</p>
      ) : (
        [...groups].map(([kind, items]) => (
          <section key={kind} className="used-group">
            <h3>
              {kind} <span className="muted">{items.length}</span>
            </h3>
            <ul>
              {items.map((item) => (
                <li key={item.id} title={item.label}>
                  {item.label || kind}
                </li>
              ))}
            </ul>
          </section>
        ))
      )}
    </aside>
  );
}

/** Threads that are running now. "Clear active" hides the current ones from this list. */
function ActiveList({ threads, onOpen }: { threads: Thread[]; onOpen: (threadId: string) => void }) {
  const [cleared, setCleared] = useState<Set<string>>(new Set());
  const running = threads.filter((item) => item.state === "running" && !item.archived_at);
  const shown = running.filter((item) => !cleared.has(item.id));
  if (shown.length === 0) return null;
  return (
    <section className="active-list">
      <header className="active-header">
        <span>{tx("Active")}</span>
        <button className="link-button" onClick={() => setCleared(new Set(running.map((item) => item.id)))}>
          {tx("Clear active")}
        </button>
      </header>
      {shown.map((item) => (
        <button key={item.id} className="active-row" onClick={() => onOpen(item.id)}>
          <span className="active-dot" aria-hidden="true" />
          <span className="active-text">
            <span className="active-title">{item.title}</span>
            <span className="muted small">{relativeTime(item.last_activity_at)}</span>
          </span>
        </button>
      ))}
    </section>
  );
}

function relativeTime(iso: string): string {
  const seconds = Math.round((new Date(iso).getTime() - Date.now()) / 1000);
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const units: [Intl.RelativeTimeFormatUnit, number][] = [["day", 86400], ["hour", 3600], ["minute", 60]];
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return rtf.format(Math.round(seconds / size), unit);
  }
  return rtf.format(0, "second");
}

/** In a code session: commit what the session changed, and push its branch. */
function CommitBar({ threadId, branch, onError }: { threadId: string; branch: string | null; onError: (message: string) => void }) {
  const [open, setOpen] = useState(false);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [pushed, setPushed] = useState<string | null>(null);

  const commit = async () => {
    setBusy(true);
    setPushed(null);
    try {
      const result = await threadApi.commit(threadId, message.trim());
      setPushed(`${tx("Pushed to")} ${result.branch} · ${result.sha.slice(0, 7)}`);
      setMessage("");
      setOpen(false);
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("Could not commit."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="commit-bar">
      <button className="used-toggle" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        {tx("Commit and push")}
      </button>
      {branch && <span className="muted small">{branch}</span>}
      {pushed && <span className="muted small">{pushed}</span>}
      {open && (
        <form
          className="commit-form"
          onSubmit={(event) => {
            event.preventDefault();
            void commit();
          }}
        >
          <input
            value={message}
            onChange={(event) => setMessage(event.target.value)}
            placeholder={tx("Commit message")}
            aria-label={tx("Commit message")}
            maxLength={500}
          />
          <button className="primary" disabled={busy || !message.trim()}>
            {tx("Commit and push")}
          </button>
        </form>
      )}
    </div>
  );
}

/** In a code session: screenshot an HTML page of the checkout, as Chromium draws it on the gateway. */
function PreviewBar({ threadId, onError }: { threadId: string; onError: (message: string) => void }) {
  const [path, setPath] = useState("index.html");
  const [busy, setBusy] = useState(false);
  const [shot, setShot] = useState<string | null>(null);

  const preview = async () => {
    setBusy(true);
    try {
      const blob = await threadApi.preview(threadId, path.trim());
      setShot((previous) => {
        if (previous) URL.revokeObjectURL(previous);
        return URL.createObjectURL(blob);
      });
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The page could not be shown."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="preview-bar">
      <form
        className="preview-form"
        onSubmit={(event) => {
          event.preventDefault();
          void preview();
        }}
      >
        <input
          value={path}
          onChange={(event) => setPath(event.target.value)}
          placeholder={tx("Page in the checkout, such as site/index.html")}
          aria-label={tx("Page to preview")}
        />
        <button className="primary" disabled={busy || !path.trim()}>
          {busy ? tx("Preview…") : tx("Preview")}
        </button>
      </form>
      {shot && (
        <figure className="preview-shot">
          <img src={shot} alt={tx("Screenshot of the page")} />
          <figcaption>
            <button className="link-button" onClick={() => {
              URL.revokeObjectURL(shot);
              setShot(null);
            }}>
              {tx("Close")}
            </button>
          </figcaption>
        </figure>
      )}
    </div>
  );
}
