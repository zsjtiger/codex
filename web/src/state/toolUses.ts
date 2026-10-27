/** A tool, search, or file action that Codex completed during a session. */
export interface ToolUse {
  id: string;
  kind: string;
  label: string;
}

/**
 * Combines the stored list with actions seen live, keeping the stored order and adding any live
 * action the server has not returned yet. Each item id appears once.
 */
export function mergeToolUses(stored: ToolUse[], live: ToolUse[]): ToolUse[] {
  const merged = [...stored];
  const seen = new Set(stored.map((use) => use.id));
  for (const use of live) {
    if (!seen.has(use.id)) {
      seen.add(use.id);
      merged.push(use);
    }
  }
  return merged;
}

/** Maps a completed app-server thread item to a tool use, or null for ordinary messages. */
export function toolUseFromItem(item: Record<string, any> | undefined): ToolUse | null {
  if (!item?.id || typeof item.type !== "string") return null;
  const id = item.id as string;
  switch (item.type) {
    case "webSearch":
      return { id, kind: "Web search", label: String(item.query ?? "") };
    case "mcpToolCall":
      return { id, kind: "Connector", label: String(item.server ?? "") };
    case "dynamicToolCall":
      return { id, kind: "Tool", label: String(item.tool ?? "") };
    case "commandExecution":
      return { id, kind: "Command", label: String(item.command ?? "") };
    case "fileChange": {
      const count = Array.isArray(item.changes) ? item.changes.length : 0;
      return { id, kind: "File change", label: `${count} file${count === 1 ? "" : "s"}` };
    }
    case "imageView":
      return { id, kind: "Image", label: String(item.path ?? "") };
    case "imageGeneration":
      return { id, kind: "Image generation", label: String(item.revisedPrompt ?? "") };
    default:
      return null;
  }
}
