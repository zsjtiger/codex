import { useEffect, useState } from "react";
import { artifacts as artifactApi, threads as threadApi } from "../api/resources";
import type { Artifact, Project, Thread, Turn, User } from "../api/types";
import type { Approval, SendOptions } from "../state/useCodex";
import { Composer } from "./Composer";
import { Conversation } from "./Conversation";
import { ChevronIcon, SparkIcon } from "./icons";

function greeting(date: Date): string {
  const hour = date.getHours();
  if (hour < 12) return "Good morning";
  if (hour < 18) return "Good afternoon";
  return "Good evening";
}

interface HomeProps {
  user: User;
  workspaceId: string;
  thread: Thread | null;
  turns: Turn[];
  agentText: Record<string, string>;
  approvals: Approval[];
  sending: boolean;
  projects: Project[];
  draftProjectId: string | null;
  onDraftProject: (projectId: string | null) => void;
  onThreadChanged: () => void;
  onSend: (text: string, options: SendOptions) => void;
  onStop: (turnId: string) => void;
  onAnswer: (approval: Approval, result: unknown) => void;
  onOpenArtifacts: () => void;
  upload: (file: File) => Promise<string>;
  error: string | null;
  onError: (message: string) => void;
  onDismissError: () => void;
}

export function Home(props: HomeProps) {
  const { user, workspaceId, thread, turns, agentText, approvals, sending, projects, draftProjectId, onDraftProject, onThreadChanged, onSend, onStop, onAnswer, onOpenArtifacts, upload, error, onError, onDismissError } = props;
  const pending = approvals.filter((a) => a.threadId === thread?.id);
  const runningTurn = [...turns].reverse().find((t) => (t.status === "running" || t.status === "queued") && t.codex_turn_id);
  const [recent, setRecent] = useState<Artifact[]>([]);

  useEffect(() => {
    if (thread) return;
    artifactApi
      .list(workspaceId, "?limit=4")
      .then(setRecent)
      .catch(() => setRecent([]));
  }, [workspaceId, thread]);

  const setThreadProject = async (projectId: string | null) => {
    if (!thread) return;
    try {
      await threadApi.setProject(thread.id, projectId);
      onThreadChanged();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not change the project.");
    }
  };

  return (
    <main className="home">
      {error && (
        <p className="banner error" role="alert">
          {error}
          <button className="banner-close" onClick={onDismissError} aria-label="Dismiss">
            ×
          </button>
        </p>
      )}

      {thread ? (
        <div className="thread-view">
          <header className="thread-title">{thread.title}</header>
          <Conversation turns={turns} agentText={agentText} approvals={pending} onAnswer={onAnswer} />
          <div className="composer-dock">
            <Composer
              onSend={onSend}
              onStop={runningTurn ? () => onStop(runningTurn.id) : undefined}
              running={Boolean(runningTurn)}
              disabled={sending}
              placeholder="Reply…"
              projects={projects}
              projectId={thread.project_id}
              onProjectChange={(id) => void setThreadProject(id)}
              upload={upload}
              onError={onError}
            />
          </div>
        </div>
      ) : (
        <div className="home-inner">
          <h1 className="greeting">
            <SparkIcon className="greeting-mark" />
            <span>{`${greeting(new Date())}, ${user.display_name}`}</span>
          </h1>
          <Composer
            onSend={onSend}
            disabled={sending}
            projects={projects}
            projectId={draftProjectId}
            onProjectChange={onDraftProject}
            upload={upload}
            onError={onError}
          />

          <section className="recent-artifacts">
            <button className="recent-heading link-button" onClick={onOpenArtifacts}>
              <span>Recent artifacts</span>
              <ChevronIcon />
            </button>
            {recent.length === 0 && <p className="empty-note">No artifacts yet</p>}
            {recent.length > 0 && (
              <div className="artifact-grid">
                {recent.map((artifact) => (
                  <button key={artifact.id} className="artifact-card" onClick={onOpenArtifacts}>
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
