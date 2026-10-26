import { artifacts } from "../data/mock";
import type { Turn, Thread, User } from "../api/types";
import type { Approval } from "../state/useCodex";
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
  thread: Thread | null;
  turns: Turn[];
  agentText: Record<string, string>;
  approvals: Approval[];
  sending: boolean;
  onSend: (text: string) => void;
  onAnswer: (approval: Approval, result: unknown) => void;
  error: string | null;
  onDismissError: () => void;
}

export function Home(props: HomeProps) {
  const { user, thread, turns, agentText, approvals, sending, onSend, onAnswer, error, onDismissError } = props;
  const pending = approvals.filter((a) => a.threadId === thread?.id);

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
            <Composer onSend={onSend} disabled={sending} placeholder="Reply…" />
          </div>
        </div>
      ) : (
        <div className="home-inner">
          <h1 className="greeting">
            <SparkIcon className="greeting-mark" />
            <span>{`${greeting(new Date())}, ${user.display_name}`}</span>
          </h1>
          <Composer onSend={onSend} disabled={sending} />

          <section className="recent-artifacts">
            <div className="recent-heading">
              <span>Recent artifacts</span>
              <span className="muted">
                <ChevronIcon />
              </span>
            </div>
            {artifacts.length === 0 && <p className="empty-note">No artifacts yet</p>}
          </section>
        </div>
      )}
    </main>
  );
}
