// Notices codex sends about the account, the configuration, and the model it is using: warnings, deprecations,
// and a model switched by codex. They are shown above the chat, not in it.

export interface Notice {
  id: number;
  text: string;
  tone: "info" | "warning";
}

let nextId = 1;

/** The notice to show for a codex notification, or null if this notification is not one to show. */
export function noticeFor(method: string, params: Record<string, unknown>): Omit<Notice, "id"> | null {
  const text = (value: unknown) => (typeof value === "string" ? value.trim() : "");
  switch (method) {
    case "warning": {
      const message = text(params.message);
      return message ? { text: message, tone: "warning" } : null;
    }
    case "configWarning":
    case "deprecationNotice": {
      const summary = text(params.summary);
      const details = text(params.details);
      if (!summary) return null;
      return { text: details ? `${summary} ${details}` : summary, tone: "warning" };
    }
    case "model/rerouted": {
      const from = text(params.fromModel);
      const to = text(params.toModel);
      return from && to ? { text: `Codex switched the model from ${from} to ${to}.`, tone: "info" } : null;
    }
    default:
      return null;
  }
}

/** A new notice with a fresh id, so the screen can dismiss it. */
export function newNotice(notice: Omit<Notice, "id">): Notice {
  return { ...notice, id: nextId++ };
}
