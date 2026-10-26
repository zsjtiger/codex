/** A tool, search, or file action that Codex completed during a session. */
export interface ToolUse {
  id: string;
  kind: string;
  label: string;
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
