// A plugin the model designs from a request, and the message that asks for it. Shared by the
// plugins page (which reads a design back into its form) and the chat (which offers to add one).

import type { PluginManifest } from "../api/types";

/** A plugin as a design: what the form needs to add it. */
export interface PluginDraft {
  name: string;
  description: string;
  manifest: PluginManifest;
}

/**
 * Reads a plugin's contents from JSON. The contents may sit under `manifest`, or at the top level
 * beside `name` and `description`. Throws when the text is not a JSON object.
 */
export function parseManifest(text: string): PluginManifest {
  const value = JSON.parse(text) as Partial<PluginManifest> & { manifest?: PluginManifest };
  if (!value || typeof value !== "object") throw new Error("not an object");
  const source = value.manifest ?? value;
  return {
    skills: Array.isArray(source.skills) ? source.skills : [],
    connectors: Array.isArray(source.connectors) ? source.connectors : [],
  };
}

/** The message that asks the model to design a plugin, in the one form the page can read back. */
export function draftPrompt(description: string): string {
  return [
    `创建插件：${description}`,
    "",
    "为 Baize 设计这个插件。插件包含若干技能（指令），以及需要时的连接器（MCP 服务器地址）。",
    "只输出一个代码块，语言标记为 plugin-manifest，内容是 JSON，格式如下，不要添加其他文字：",
    '{"name": "插件名", "description": "一句话说明", "skills": [{"name": "技能名", "description": "说明", "instructions": "给模型的指令"}], "connectors": [{"name": "连接器名", "url": "https://..."}]}',
    "只有需求明确需要外部服务时才添加连接器。",
  ].join("\n");
}

/** The plugin a reply describes, from its plugin-manifest block. */
export function parsePluginDraft(reply: string): PluginDraft | null {
  const block = /```plugin-manifest\s*\n([\s\S]*?)```/.exec(reply);
  if (!block) return null;
  try {
    const value = JSON.parse(block[1]) as { name?: unknown; description?: unknown };
    const manifest = parseManifest(block[1]);
    if (typeof value.name !== "string" || !value.name.trim()) return null;
    return {
      name: value.name.trim(),
      description: typeof value.description === "string" ? value.description : "",
      manifest,
    };
  } catch {
    return null;
  }
}

