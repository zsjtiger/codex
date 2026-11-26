import { useEffect, useState } from "react";
import { threads as threadApi } from "../api/resources";
import type { Goal } from "../api/types";
import { tx } from "../i18n";
import { subscribeThread } from "../state/threadEvents";

/** What codex calls each goal status, as a short label. */
const STATUS_LABEL: Record<string, string> = {
  active: "Working",
  paused: "Paused",
  blocked: "Blocked",
  usageLimited: "Usage limit reached",
  budgetLimited: "Budget reached",
  complete: "Complete",
};

/**
 * The chat's goal: what codex is working toward, how far it has got, and the budget it may use. Codex keeps
 * working toward an active goal on its own, so changes are made here deliberately.
 */
export function GoalBar({ threadId, onError }: { threadId: string; onError: (message: string) => void }) {
  const [goal, setGoal] = useState<Goal | null>(null);
  const [loaded, setLoaded] = useState(false);

  const read = () =>
    threadApi
      .goal(threadId)
      .then((next) => setGoal(next))
      .catch(() => setGoal(null))
      .finally(() => setLoaded(true));

  useEffect(() => {
    setLoaded(false);
    void read();
    // codex announces a change or a clear; the bar reads the goal again.
    return subscribeThread(threadId, () => void read());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [threadId]);

  const change = async (body: { objective?: string; status?: string; token_budget?: number | null }) => {
    try {
      setGoal(await threadApi.setGoal(threadId, body));
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The goal could not be changed."));
    }
  };

  const edit = () => {
    const objective = window.prompt(tx("What should codex work toward?"), goal?.objective ?? "");
    if (objective === null) return;
    if (!objective.trim()) {
      onError(tx("A goal needs an objective."));
      return;
    }
    const budget = window.prompt(tx("Token budget (leave empty for none)"), goal?.token_budget?.toString() ?? "");
    if (budget === null) return;
    const trimmed = budget.trim();
    const parsed = trimmed === "" ? null : Number(trimmed);
    if (parsed !== null && (!Number.isInteger(parsed) || parsed <= 0)) {
      onError(tx("The token budget is a positive whole number."));
      return;
    }
    void change({ objective: objective.trim(), token_budget: parsed });
  };

  const clear = async () => {
    if (!window.confirm(tx("Clear this goal?"))) return;
    try {
      await threadApi.clearGoal(threadId);
      setGoal(null);
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("The goal could not be cleared."));
    }
  };

  if (!loaded) return null;
  if (!goal) {
    return (
      <div className="goal-bar">
        <button type="button" className="link-button" onClick={edit}>
          {tx("Set a goal")}
        </button>
      </div>
    );
  }
  const paused = goal.status === "paused";
  return (
    <div className="goal-bar">
      <div className="goal-main">
        <span className="goal-objective">{goal.objective}</span>
        <span className={`goal-status goal-${goal.status}`}>{tx(STATUS_LABEL[goal.status] ?? goal.status)}</span>
        <span className="muted small">
          {goal.tokens_used.toLocaleString()} {tx("tokens")}
          {goal.token_budget !== null && ` / ${goal.token_budget.toLocaleString()}`} · {Math.round(goal.time_used_seconds / 60)} {tx("min")}
        </span>
      </div>
      <div className="goal-actions">
        {(goal.status === "active" || goal.status === "paused") && (
          <button type="button" className="link-button" onClick={() => void change({ status: paused ? "active" : "paused" })}>
            {paused ? tx("Resume") : tx("Pause")}
          </button>
        )}
        <button type="button" className="link-button" onClick={edit}>
          {tx("Edit")}
        </button>
        <button type="button" className="link-button" onClick={() => void clear()}>
          {tx("Clear")}
        </button>
      </div>
    </div>
  );
}
