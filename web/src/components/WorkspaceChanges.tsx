import { useState } from "react";
import { ApiError } from "../api/client";
import { threads as threadApi } from "../api/resources";
import type { WorkspaceDiff } from "../api/types";
import { tx } from "../i18n";
import { TurnChanges } from "./Conversation";

/**
 * Everything in the code chat's checkout that differs from the remote's copy of its branch, committed or not.
 * It covers the whole checkout, not one turn, so it can include changes made outside Codex's turns.
 */
export function WorkspaceChanges({ threadId, onError }: { threadId: string; onError: (message: string) => void }) {
  const [found, setFound] = useState<WorkspaceDiff | null>(null);
  const [loading, setLoading] = useState(false);
  const [missing, setMissing] = useState(false);

  const load = async () => {
    setLoading(true);
    try {
      setFound(await threadApi.workspaceDiff(threadId));
      setMissing(false);
    } catch (err) {
      setFound(null);
      if (err instanceof ApiError && err.status === 404) {
        setMissing(true);
      } else {
        onError(err instanceof Error ? err.message : tx("The changes could not be read."));
      }
    } finally {
      setLoading(false);
    }
  };

  return (
    <section className="codex-skills">
      <div className="live-check">
        <h2 className="codex-skills-title">{tx("Workspace changes")}</h2>
        <button type="button" className="link-button" disabled={loading} onClick={() => void load()}>
          {loading ? tx("Reading…") : found ? tx("Read again") : tx("Read changes")}
        </button>
        {found && <span className="muted small">{tx("Compared with")} {found.sha.slice(0, 7)}</span>}
      </div>
      {missing && <p className="muted small">{tx("This checkout has no changes codex can compare.")}</p>}
      {found && found.diff.trim() === "" && <p className="muted small">{tx("No changes since the remote's copy of the branch.")}</p>}
      {found && found.diff.trim() !== "" && <TurnChanges diff={found.diff} defaultOpen />}
    </section>
  );
}
