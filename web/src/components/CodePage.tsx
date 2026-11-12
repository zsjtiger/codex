import { useEffect, useState } from "react";
import { github as githubApi, repos as repoApi, threads as threadApi } from "../api/resources";
import type { CodeRepo, GitHubRepo } from "../api/types";
import { tx } from "../i18n";

function errorText(err: unknown): string {
  return err instanceof Error ? err.message : tx("Something went wrong.");
}

/** Messages for the result GitHub's callback leaves in the address. */
const outcomes: Record<string, string> = {
  connected: "GitHub account connected.",
  denied: "GitHub access was not approved.",
  expired: "That sign-in link has expired. Try connecting again.",
  error: "GitHub could not be connected. Try again.",
};

interface CodePageProps {
  workspaceId: string;
  environmentId: string | null;
  onOpenThread: (threadId: string) => void;
  onError: (message: string) => void;
}

/** Code's home: connect GitHub, pick a repository and branch, and start sessions on linked repositories. */
export function CodePage({ workspaceId, environmentId, onOpenThread, onError }: CodePageProps) {
  const [account, setAccount] = useState<{ connected: boolean; login: string | null } | null>(null);
  const [linked, setLinked] = useState<CodeRepo[] | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [available, setAvailable] = useState<GitHubRepo[] | null>(null);
  const [picked, setPicked] = useState<GitHubRepo | null>(null);
  const [branches, setBranches] = useState<string[]>([]);
  const [branch, setBranch] = useState("");
  const [busy, setBusy] = useState(false);
  const [gitUrl, setGitUrl] = useState("");
  const [gitUser, setGitUser] = useState("");
  const [gitToken, setGitToken] = useState("");
  const [probed, setProbed] = useState<{ name: string | null; default_branch: string | null; branches: string[] } | null>(null);
  const [gitBranch, setGitBranch] = useState("");

  const loadLinked = () =>
    repoApi
      .list(workspaceId)
      .then(setLinked)
      .catch((err: unknown) => onError(errorText(err)));

  const loadAccount = () =>
    githubApi
      .status()
      .then(setAccount)
      .catch((err: unknown) => onError(errorText(err)));

  // Returning from GitHub: show the outcome once, then clean the address.
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const outcome = params.get("github");
    if (outcome) {
      setNotice(tx(outcomes[outcome] ?? "GitHub could not be connected. Try again."));
      window.history.replaceState(null, "", window.location.pathname);
    }
    void loadAccount();
    void loadLinked();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  // Repositories the account can reach, filtered as the person types.
  useEffect(() => {
    if (!account?.connected) return;
    const timer = window.setTimeout(() => {
      githubApi
        .repos(search)
        .then(setAvailable)
        .catch((err: unknown) => onError(errorText(err)));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [account?.connected, search]);

  const connect = async () => {
    setBusy(true);
    try {
      const { url } = await githubApi.connect();
      window.location.href = url;
    } catch (err) {
      setBusy(false);
      onError(errorText(err));
    }
  };

  const disconnect = async () => {
    if (!window.confirm(tx("Disconnect your GitHub account? Sessions keep their history."))) return;
    try {
      await githubApi.disconnect();
      setPicked(null);
      setAvailable(null);
      await loadAccount();
    } catch (err) {
      onError(errorText(err));
    }
  };

  const pick = async (repo: GitHubRepo) => {
    setPicked(repo);
    setBranch(repo.default_branch);
    setBranches([]);
    try {
      const names = await githubApi.branches(repo.full_name);
      setBranches(names);
      if (!names.includes(repo.default_branch) && names.length > 0) setBranch(names[0]);
    } catch (err) {
      onError(errorText(err));
    }
  };

  const addRepository = async () => {
    if (!picked) return;
    setBusy(true);
    try {
      await repoApi.link(workspaceId, { full_name: picked.full_name, default_branch: branch || picked.default_branch });
      setPicked(null);
      await loadLinked();
    } catch (err) {
      onError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  const checkAddress = async () => {
    setBusy(true);
    try {
      const found = await repoApi.probe(workspaceId, {
        url: gitUrl.trim(),
        username: gitUser.trim() || undefined,
        token: gitToken.trim() || undefined,
      });
      setProbed(found);
      setGitBranch(found.default_branch ?? found.branches[0] ?? "main");
    } catch (err) {
      setProbed(null);
      onError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  const addByAddress = async () => {
    setBusy(true);
    try {
      await repoApi.link(workspaceId, {
        url: gitUrl.trim(),
        username: gitUser.trim() || undefined,
        token: gitToken.trim() || undefined,
        default_branch: gitBranch,
      });
      setGitUrl("");
      setGitUser("");
      setGitToken("");
      setProbed(null);
      await loadLinked();
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
      await loadLinked();
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
      {notice && <p className="code-notice muted small">{notice}</p>}

      {account === null && <p className="muted small">{tx("Loading…")}</p>}
      {account && !account.connected && (
        <div className="code-connect-github">
          <button className="primary" onClick={() => void connect()} disabled={busy}>
            {tx("Connect GitHub")}
          </button>
          <span className="muted small">{tx("Sign in with GitHub to pick from the repositories you can reach.")}</span>
        </div>
      )}

      {account?.connected && (
        <section className="code-picker">
          <div className="code-account">
            <span className="muted small">
              {tx("Signed in to GitHub as")} <strong>{account.login}</strong>
            </span>
            <button className="link-button" onClick={() => void disconnect()}>
              {tx("Disconnect")}
            </button>
          </div>
          <input
            className="code-search"
            placeholder={tx("Search your repositories")}
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            aria-label={tx("Search your repositories")}
          />
          <div className="code-available">
            {available === null && <p className="muted small">{tx("Loading…")}</p>}
            {available?.length === 0 && <p className="muted small">{tx("No matching repositories.")}</p>}
            {available?.slice(0, 50).map((repo) => (
              <button
                key={repo.full_name}
                className={`code-available-row ${picked?.full_name === repo.full_name ? "code-available-on" : ""}`}
                onClick={() => void pick(repo)}
              >
                <span>{repo.full_name}</span>
                <span className="muted small">{repo.private ? tx("Private") : tx("Public")}</span>
              </button>
            ))}
          </div>
          {picked && (
            <div className="code-add">
              <strong>{picked.full_name}</strong>
              <label className="muted small">
                {tx("Branch")}{" "}
                <select value={branch} onChange={(event) => setBranch(event.target.value)} aria-label={tx("Branch")}>
                  {(branches.length > 0 ? branches : [picked.default_branch]).map((name) => (
                    <option key={name} value={name}>
                      {name}
                    </option>
                  ))}
                </select>
              </label>
              <button className="primary" onClick={() => void addRepository()} disabled={busy}>
                {tx("Add repository")}
              </button>
            </div>
          )}
        </section>
      )}

      <section className="code-picker">
        <h2>{tx("Add a git repository by address")}</h2>
        <p className="muted small">
          {tx("GitLab, Gitea or any git server. Use an https address and an access token that can read and write.")}
        </p>
        <input
          className="code-search"
          placeholder="https://gitlab.example.com/group/project.git"
          value={gitUrl}
          onChange={(event) => {
            setGitUrl(event.target.value);
            setProbed(null);
          }}
          aria-label={tx("Git address")}
        />
        <input
          className="code-search"
          placeholder={tx("User name (optional)")}
          value={gitUser}
          onChange={(event) => setGitUser(event.target.value)}
          aria-label={tx("User name")}
        />
        <input
          className="code-search"
          type="password"
          autoComplete="off"
          placeholder={tx("Access token")}
          value={gitToken}
          onChange={(event) => setGitToken(event.target.value)}
          aria-label={tx("Access token")}
        />
        <div>
          <button className="primary" onClick={() => void checkAddress()} disabled={busy || !gitUrl.trim()}>
            {tx("Check address")}
          </button>
        </div>
        {probed && (
          <div className="code-add">
            <strong>{probed.name ?? gitUrl.trim()}</strong>
            <label className="muted small">
              {tx("Branch")}{" "}
              <select value={gitBranch} onChange={(event) => setGitBranch(event.target.value)} aria-label={tx("Branch")}>
                {(probed.branches.length > 0 ? probed.branches : [gitBranch]).map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </label>
            <button className="primary" onClick={() => void addByAddress()} disabled={busy}>
              {tx("Add repository")}
            </button>
          </div>
        )}
      </section>

      <section className="code-linked">
        <h2>{tx("Repositories")}</h2>
        {linked === null && <p className="muted small">{tx("Loading…")}</p>}
        {linked?.length === 0 && <p className="muted">{tx("No repositories added yet.")}</p>}
        {linked?.map((repo) => (
          <div className="code-repo" key={repo.id}>
            <div className="code-repo-info">
              <strong>{repo.full_name}</strong>
              <span className="muted small">
                {repo.default_branch} · {repo.by_url ? tx("Git address") : repo.has_token ? tx("Uses your GitHub account") : tx("Public")}
              </span>
            </div>
            <div className="code-repo-actions">
              <button className="primary" onClick={() => void startSession(repo)}>
                {tx("New session")}
              </button>
              <button className="link-button" onClick={() => void remove(repo)}>
                {tx("Remove")}
              </button>
            </div>
          </div>
        ))}
      </section>
    </main>
  );
}
