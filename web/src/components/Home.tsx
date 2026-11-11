import { useEffect, useState } from "react";
import { artifacts as artifactApi, skills as skillApi, threads as threadApi } from "../api/resources";
import type { Artifact, Environment, Project, Skill, Thread, Turn, User } from "../api/types";
import type { Activity, Approval, SendOptions } from "../state/useCodex";
import type { ToolUse } from "../state/toolUses";
import { Composer } from "./Composer";
import { usePreview } from "./Preview";
import { Conversation } from "./Conversation";
import { ChevronIcon, SparkIcon } from "./icons";
import { useI18n } from "../i18n";
import { tx } from "../i18n";

interface HomeProps {
  user: User;
  workspaceId: string;
  thread: Thread | null;
  threads: Thread[];
  onOpenThread: (threadId: string) => void;
  turns: Turn[];
  agentText: Record<string, string>;
  toolUses: Record<string, ToolUse[]>;
  environment: Environment | null;
  activity: Record<string, Activity>;
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
}

export function Home(props: HomeProps) {
  const { t } = useI18n();
  const { user, workspaceId, thread, threads, onOpenThread, turns, agentText, toolUses, activity, environment, approvals, sending, projects, draftProjectId, onDraftProject, onCreateProject, onThreadChanged, onSend, onStop, onAnswer, onFeedback, onOpenArtifacts, upload, error, onError, onDismissError } = props;
  const pending = approvals.filter((a) => a.threadId === thread?.id);
  const runningTurn = [...turns].reverse().find((t) => (t.status === "running" || t.status === "queued") && t.codex_turn_id);
  const [recent, setRecent] = useState<Artifact[]>([]);
  const { open } = usePreview();
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

  useEffect(() => {
    if (thread) return;
    artifactApi
      .list(workspaceId, "?limit=4")
      .then(setRecent)
      .catch(() => setRecent([]));
  }, [workspaceId, thread]);

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
          <div className="thread-scroll">
            <Conversation
              turns={turns}
              agentText={agentText}
              activity={activity}
              approvals={pending}
              onAnswer={onAnswer}
              onFeedback={onFeedback}
              onRegenerate={(input) => onSend(input, {})}
            />
          </div>
          <div className="composer-dock">
            <Composer
              environmentId={environment?.id ?? null}
              onSend={onSend}
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
