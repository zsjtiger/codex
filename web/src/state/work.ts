// What Codex is doing while it writes code: shell commands with their live output, file changes
// with their diffs, and the plan. Built from the app-server's events, keyed by Codex turn id.

export interface WorkFile {
  path: string;
  /** "add", "delete" or "update". */
  kind: string;
  diff: string;
}

export interface WorkItem {
  id: string;
  type: "command" | "file";
  command: string;
  /** Live stdout and stderr, appended as they arrive. */
  output: string;
  /** "inProgress", "completed", "failed" or "declined". */
  status: string;
  exitCode: number | null;
  durationMs: number | null;
  files: WorkFile[];
}

export interface PlanStep {
  step: string;
  status: string;
}

export interface WorkLog {
  plan: { explanation: string | null; steps: PlanStep[] } | null;
  /** Item ids in the order they started. */
  order: string[];
  items: Record<string, WorkItem>;
}

export const WORK_METHODS = new Set([
  "item/started",
  "item/completed",
  "item/commandExecution/outputDelta",
  "turn/plan/updated",
]);

function emptyLog(): WorkLog {
  return { plan: null, order: [], items: {} };
}

/** A patch kind arrives as a tagged object, e.g. { type: "update", movePath: null }. */
function kindName(kind: unknown): string {
  if (typeof kind === "string") return kind;
  if (kind && typeof kind === "object" && "type" in kind) return String((kind as { type: unknown }).type);
  return "update";
}

function upsert(log: WorkLog, id: string, type: WorkItem["type"], base: Partial<WorkItem>): WorkLog {
  const existing = log.items[id];
  const defaults: WorkItem = {
    id,
    type,
    command: "",
    output: "",
    status: "inProgress",
    exitCode: null,
    durationMs: null,
    files: [],
  };
  const item: WorkItem = {
    ...defaults,
    ...existing,
    // Undefined fields mean "unchanged", so they must not overwrite what is already known.
    ...(Object.fromEntries(Object.entries(base).filter(([, value]) => value !== undefined)) as Partial<WorkItem>),
  };
  return {
    ...log,
    order: existing ? log.order : [...log.order, id],
    items: { ...log.items, [id]: item },
  };
}

/**
 * Applies one app-server event to a turn's work log. Returns the log unchanged for events that
 * do not affect it.
 */
export function applyWorkEvent(current: WorkLog | undefined, method: string, params: Record<string, any>): WorkLog {
  const log = current ?? emptyLog();
  switch (method) {
    case "turn/plan/updated":
      return { ...log, plan: { explanation: params.explanation ?? null, steps: (params.plan ?? []) as PlanStep[] } };
    case "item/commandExecution/outputDelta": {
      const id = String(params.itemId ?? "");
      if (!id) return log;
      const existing = log.items[id];
      return upsert(log, id, "command", { output: (existing?.output ?? "") + String(params.delta ?? "") });
    }
    case "item/started":
    case "item/completed": {
      const item = params.item as Record<string, any> | undefined;
      if (!item?.id) return log;
      const id = String(item.id);
      if (item.type === "commandExecution") {
        return upsert(log, id, "command", {
          command: String(item.command ?? ""),
          // The aggregated output arrives with the completed item; until then, keep the streamed text.
          output: typeof item.aggregatedOutput === "string" ? item.aggregatedOutput : undefined,
          status: String(item.status ?? (method === "item/completed" ? "completed" : "inProgress")),
          exitCode: typeof item.exitCode === "number" ? item.exitCode : null,
          durationMs: typeof item.durationMs === "number" ? item.durationMs : null,
        });
      }
      if (item.type === "fileChange") {
        const files = (Array.isArray(item.changes) ? item.changes : []).map((change: Record<string, any>) => ({
          path: String(change.path ?? ""),
          kind: kindName(change.kind),
          diff: String(change.diff ?? ""),
        }));
        return upsert(log, id, "file", {
          files,
          status: String(item.status ?? (method === "item/completed" ? "completed" : "inProgress")),
        });
      }
      return log;
    }
    default:
      return log;
  }
}
