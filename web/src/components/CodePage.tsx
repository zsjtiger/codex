import { useEffect, useState } from "react";
import { repos as repoApi, threads as threadApi } from "../api/resources";
import type { CodeRepo } from "../api/types";
import { tx } from "../i18n";

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : tx("Something went wrong.");
}

interface CodePageProps {
  workspaceId: string;
  isAdmin: boolean;
  environmentId: string | null;
  onOpenThread: (threadId: string) => void;
  onError: (message: string) => void;
}

/** The code mode's home: linked GitHub repositories, and the form to link another. */
export function CodePage({ workspaceId, isAdmin, environmentId, onOpenThread, onError }: CodePageProps) {
  const [list, setList] = useState<CodeRepo[] | null>(null);
  const [fullName, setFullName] = useState("");
  const [branch, setBranch] = useState("");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);

  const load = () =>
    repoApi
      .list(workspaceId)
      .then(setList)
      .catch((err: unknown) => onError(errorText(err)));

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  const connect = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    try {
      await repoApi.link(workspaceId, {
        full_name: fullName.trim(),
        default_branch: branch.trim() || undefined,
        token: token.trim() || undefined,
      });
      setFullName("");
      setBranch("");
      setToken("");
      await load();
    } catch (err) {
      onError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  const startSession = async (repo: CodeRepo) => {
    if (!environmentId) {
      onError(tx("No environment is ready yet."));
      return;
    }
    try {
      const thread = await threadApi.create(workspaceId, {
        title: repo.full_name,
        environment_id: environmentId,
        repo_id: repo.id,
      });
      onOpenThread(thread.id);
    } catch (err) {
      onError(errorText(err));
    }
  };

  const remove = async (repo: CodeRepo) => {
    if (!window.confirm(tx("Remove this repository? Sessions that used it keep their history."))) return;
    try {
      await repoApi.remove(repo.id);
      await load();
    } catch (err) {
      onError(errorText(err));
    }
  };

  return (
    <main className="code-page">
      <h1 className="code-title">{tx("Code")}</h1>
      <p className="muted">
        {tx("Connect a GitHub repository. Each session works on its own branch, and you choose when to commit and push.")}
      </p>

      {list === null && <p className="muted small">{tx("Loading…")}</p>}
      {list?.length === 0 && <p className="muted">{tx("No repositories connected yet.")}</p>}
      {list?.map((repo) => (
        <div className="code-repo" key={repo.id}>
          <div className="code-repo-info">
            <strong>{repo.full_name}</strong>
            <span className="muted small">
              {repo.default_branch} · {repo.has_token ? tx("Private access") : tx("Public")}
            </span>
          </div>
          <div className="code-repo-actions">
            <button className="primary" onClick={() => void startSession(repo)}>
              {tx("New session")}
            </button>
            {isAdmin && (
              <button className="link-button" onClick={() => void remove(repo)}>
                {tx("Remove")}
              </button>
            )}
          </div>
        </div>
      ))}

      {isAdmin && (
        <form className="code-connect" onSubmit={connect}>
          <h2>{tx("Connect a repository")}</h2>
          <input
            required
            placeholder="owner/repository"
            value={fullName}
            onChange={(event) => setFullName(event.target.value)}
            aria-label={tx("Repository")}
          />
          <input
            placeholder={tx("Default branch (main)")}
            value={branch}
            onChange={(event) => setBranch(event.target.value)}
            aria-label={tx("Default branch")}
          />
          <input
            type="password"
            autoComplete="off"
            placeholder={tx("Access token (optional for public repositories)")}
            value={token}
            onChange={(event) => setToken(event.target.value)}
            aria-label={tx("Access token")}
          />
          <p className="muted small">
            {tx("The repository is cloned when you connect it. The token is stored encrypted and is never shown again.")}
          </p>
          <button className="primary" disabled={busy || !fullName.trim()}>
            {tx("Connect")}
          </button>
        </form>
      )}
    </main>
  );
}
