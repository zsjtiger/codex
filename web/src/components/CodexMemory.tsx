import { useEffect, useState } from "react";
import { environments } from "../api/resources";
import type { CodexMemory as CodexMemoryState, Environment } from "../api/types";
import { tx } from "../i18n";

/** One environment's codex memory: how many past threads codex has learned from, and whether it is ready. */
function EnvironmentMemory({ environment }: { environment: Environment }) {
  const [memory, setMemory] = useState<CodexMemoryState | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    environments
      .codexMemory(environment.id)
      .then((next) => current && setMemory(next))
      .catch((err) => current && setError(err instanceof Error ? err.message : tx("Could not read codex's memory.")));
    return () => {
      current = false;
    };
  }, [environment.id]);

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
    </div>
  );
}

/**
 * Codex's own memory for each environment of the workspace. It is separate from the notes a person keeps
 * here: codex learns from past threads on its own.
 */
export function CodexMemoryList({ workspaceId }: { workspaceId: string | null }) {
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
        <EnvironmentMemory key={environment.id} environment={environment} />
      ))}
    </>
  );
}
