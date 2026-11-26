import { useEffect, useState } from "react";
import { environments } from "../api/resources";
import type { CodexMemory as CodexMemoryState, Environment } from "../api/types";
import { tx } from "../i18n";

/** One environment's codex memory: how many past threads codex has learned from, and whether it is ready. */
function EnvironmentMemory({ environment, isAdmin }: { environment: Environment; isAdmin: boolean }) {
  const [memory, setMemory] = useState<CodexMemoryState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const read = () =>
    environments
      .codexMemory(environment.id)
      .then((next) => setMemory(next))
      .catch((err) => setError(err instanceof Error ? err.message : tx("Could not read codex's memory.")));

  useEffect(() => {
    let current = true;
    void environments
      .codexMemory(environment.id)
      .then((next) => current && setMemory(next))
      .catch((err) => current && setError(err instanceof Error ? err.message : tx("Could not read codex's memory.")));
    return () => {
      current = false;
    };
  }, [environment.id]);

  const reset = async () => {
    // Clearing codex's memory cannot be undone, and it affects every chat in the environment.
    if (!window.confirm(tx("Clear codex's memory for this environment? This cannot be undone, and it affects every chat here."))) return;
    setBusy(true);
    try {
      await environments.resetCodexMemory(environment.id);
      await read();
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : tx("Could not reset codex's memory."));
    } finally {
      setBusy(false);
    }
  };

  let state = tx("Loading…");
  if (error) state = error;
  else if (memory) {
    state = memory.ready
      ? `${tx("Ready")} · ${memory.consolidated_threads} ${tx(memory.consolidated_threads === 1 ? "past thread" : "past threads")}`
      : tx("Not ready yet. Codex learns from past threads in the background.");
  }
  return (
    <div className="settings-row">
      <div>
        <div className="detail-label">{environment.name}</div>
        <div className="muted small">{state}</div>
      </div>
      {isAdmin && memory && (
        <button type="button" className="link-button" disabled={busy} onClick={() => void reset()}>
          {busy ? tx("Clearing…") : tx("Clear memory")}
        </button>
      )}
    </div>
  );
}

/**
 * Codex's own memory for each environment of the workspace. It is separate from the notes a person keeps
 * here: codex learns from past threads on its own.
 */
export function CodexMemoryList({ workspaceId, isAdmin }: { workspaceId: string | null; isAdmin: boolean }) {
  const [envs, setEnvs] = useState<Environment[] | null>(null);
  useEffect(() => {
    if (!workspaceId) return;
    let current = true;
    environments
      .list(workspaceId)
      .then((next) => current && setEnvs(next))
      .catch(() => current && setEnvs([]));
    return () => {
      current = false;
    };
  }, [workspaceId]);
  if (!workspaceId || !envs || envs.length === 0) return null;
  return (
    <>
      <div className="detail-label settings-subheading">{tx("Codex memory")}</div>
      {envs.map((environment) => (
        <EnvironmentMemory key={environment.id} environment={environment} isAdmin={isAdmin} />
      ))}
    </>
  );
}
