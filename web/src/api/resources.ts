import { apiUrl } from "./config";
// Typed calls for the gateway's resource routes. Each function maps to one route in
// gateway/src/routes; see the gateway README for the full list.

import { api, ApiError, getToken } from "./client";
import type {
  Artifact,
  ArtifactKind,
  Attachment,
  MemoryImportResult,
  AuditEntry,
  Member,
  Workspace,
  FromChatResult,
  MemoryNote,
  MemoryView,
  ProjectNote,
  ProjectFileHit,
  ScheduledRun,
  Connector,
  Frequency,
  Project,
  ScheduledTask,
  SearchResults,
  Skill,
  Thread,
  ToolUseRecord,
  Turn,
  UsageReport,
} from "./types";

export const projects = {
  list: (ws: string) => api<Project[]>("GET", `/workspaces/${ws}/projects`),
  create: (ws: string, body: { name: string; instructions: string }) =>
    api<Project>("POST", `/workspaces/${ws}/projects`, body),
  update: (id: string, body: { name?: string; instructions?: string }) =>
    api<Project>("PATCH", `/projects/${id}`, body),
  remove: (id: string) => api<void>("DELETE", `/projects/${id}`),
};

export const skills = {
  list: (ws: string, kind: "skill" | "plugin") => api<Skill[]>("GET", `/workspaces/${ws}/skills?kind=${kind}`),
  create: (ws: string, body: { kind: "skill" | "plugin"; name: string; description: string; instructions: string }) =>
    api<Skill>("POST", `/workspaces/${ws}/skills`, body),
  update: (id: string, body: { enabled?: boolean; name?: string; description?: string; instructions?: string }) =>
    api<Skill>("PATCH", `/skills/${id}`, body),
  remove: (id: string) => api<void>("DELETE", `/skills/${id}`),
};

export const artifacts = {
  list: (ws: string, query = "") => api<Artifact[]>("GET", `/workspaces/${ws}/artifacts${query}`),
  create: (ws: string, body: { title: string; kind: ArtifactKind; content?: string; project_id?: string }) =>
    api<Artifact>("POST", `/workspaces/${ws}/artifacts`, body),
  update: (id: string, body: { title?: string; content?: string; project_id?: string | null }) =>
    api<Artifact>("PATCH", `/artifacts/${id}`, body),
  remove: (id: string) => api<void>("DELETE", `/artifacts/${id}`),
};

export interface TaskInput {
  name: string;
  prompt: string;
  frequency: Frequency;
  time_of_day: string;
  weekday?: number | null;
  project_id?: string | null;
}

export const members = {
  list: (ws: string) => api<Member[]>("GET", `/workspaces/${ws}/members`),
  add: (ws: string, body: { email: string; role: "admin" | "member" }) => api<Member>("POST", `/workspaces/${ws}/members`, body),
  changeRole: (ws: string, userId: string, role: "admin" | "member") =>
    api<Member>("PATCH", `/workspaces/${ws}/members/${userId}`, { role }),
  remove: (ws: string, userId: string) => api<void>("DELETE", `/workspaces/${ws}/members/${userId}`),
};

/** Single-use invite links for a workspace. */
export const invites = {
  create: (ws: string, role: "admin" | "member") =>
    api<{ token: string; role: string; expires_at: string }>("POST", `/workspaces/${ws}/invites`, { role }),
  preview: (token: string) => api<{ workspace_name: string; role: string; expires_at: string }>("GET", `/invites/${token}`),
  accept: (token: string) => api<{ workspace_id: string }>("POST", `/invites/${token}/accept`),
};

export const workspaceSettings = {
  update: (id: string, body: { name: string }) => api<Workspace>("PATCH", `/workspaces/${id}`, body),
  remove: (id: string) => api<void>("DELETE", `/workspaces/${id}`),
};

export const audit = {
  list: (ws: string, offset: number, limit = 50) =>
    api<AuditEntry[]>("GET", `/workspaces/${ws}/audit-logs?limit=${limit}&offset=${offset}`),
};

/** Notes kept for a project; they are added to the project's new chats. */
export const projectMemory = {
  list: (projectId: string) => api<ProjectNote[]>("GET", `/projects/${projectId}/memory`),
  add: (projectId: string, content: string) => api<ProjectNote>("POST", `/projects/${projectId}/memory`, { content }),
  remove: (projectId: string, noteId: string) => api<void>("DELETE", `/projects/${projectId}/memory/${noteId}`),
};

export const memory = {
  get: () => api<MemoryView>("GET", "/me/memory"),
  add: (content: string) => api<MemoryNote>("POST", "/me/memory", { content }),
  edit: (id: string, content: string) => api<MemoryNote>("PATCH", `/me/memory/${id}`, { content }),
  remove: (id: string) => api<void>("DELETE", `/me/memory/${id}`),
  setEnabled: (enabled: boolean) => api<void>("PUT", "/me/memory/enabled", { enabled }),
  setSearchChats: (enabled: boolean) => api<void>("PUT", "/me/memory/search-chats", { enabled }),
  /** Starts reading one chat for durable facts; returns the helper chat to poll. */
  startFromChat: (threadId: string) => api<{ helper: string }>("POST", "/me/memory/from-chat", { thread_id: threadId }),
  fromChatResult: (helper: string) => api<FromChatResult>("GET", `/me/memory/from-chat/${helper}`),
  /** Imports notes from pasted text (a string) or a file (a zip, .md or .txt). */
  importNotes: async (input: string | File): Promise<MemoryImportResult> => {
    const headers: Record<string, string> = { "content-type": typeof input === "string" ? "text/plain" : contentTypeOf(input) };
    const token = getToken();
    if (token) headers.authorization = `Bearer ${token}`;
    const response = await fetch(apiUrl("/me/memory/import"), { method: "POST", headers, body: input });
    if (!response.ok) {
      const data = await response.json().catch(() => null);
      throw new ApiError(response.status, data?.error?.code ?? "error", data?.error?.message ?? response.statusText);
    }
    return response.json();
  },
};

export const scheduled = {
  list: (ws: string) => api<ScheduledTask[]>("GET", `/workspaces/${ws}/scheduled-tasks`),
  create: (ws: string, body: TaskInput) => api<ScheduledTask>("POST", `/workspaces/${ws}/scheduled-tasks`, body),
  update: (id: string, body: Partial<TaskInput> & { enabled?: boolean }) =>
    api<ScheduledTask>("PATCH", `/scheduled-tasks/${id}`, body),
  remove: (id: string) => api<void>("DELETE", `/scheduled-tasks/${id}`),
  runs: (id: string) => api<ScheduledRun[]>("GET", `/scheduled-tasks/${id}/runs`),
  runNow: (id: string) => api<{ thread_id: string }>("POST", `/scheduled-tasks/${id}/run`),
};

export const search = (ws: string, q: string) =>
  api<SearchResults>("GET", `/workspaces/${ws}/search?q=${encodeURIComponent(q)}`);

/** A model the environment's codex runtime offers. */
export interface ModelOption {
  model: string;
  display_name: string;
  is_default: boolean;
  efforts: string[];
}

export const environments = {
  models: (id: string) => api<ModelOption[]>("GET", `/environments/${id}/models`),
};

export const threads = {
  create: (ws: string, body: { title: string; environment_id: string; project_id?: string | null }) =>
    api<Thread>("POST", `/workspaces/${ws}/threads`, body),
  setProject: (id: string, projectId: string | null) =>
    api<Thread>("PATCH", `/threads/${id}`, { project_id: projectId }),
  rename: (id: string, title: string) => api<Thread>("PATCH", `/threads/${id}`, { title }),
  archive: (id: string) => api<Thread>("PATCH", `/threads/${id}`, { archived: true }),
  remove: (id: string) => api<void>("DELETE", `/threads/${id}`),
  /** Saves a snapshot of the chat as a Markdown file in a project. */
  saveToProject: (id: string, projectId: string) =>
    api<Attachment>("POST", `/threads/${id}/save-to-project`, { project_id: projectId }),
  turns: (id: string) => api<Turn[]>("GET", `/threads/${id}/turns?limit=100`),
  toolUses: (id: string) => api<ToolUseRecord[]>("GET", `/threads/${id}/tool-uses?limit=100`),
  send: (id: string, body: { input: string; model?: string; effort?: string; attachment_ids?: string[]; skill_ids?: string[] }) =>
    api<Turn>("POST", `/threads/${id}/turns`, body),
};

export const interruptTurn = (id: string) => api<void>("POST", `/turns/${id}/interrupt`);

/** Rates a reply. `null` clears the rating. */
export const rateTurn = (id: string, rating: "up" | "down" | null) => api<void>("PUT", `/turns/${id}/feedback`, { rating });

/** The content type to record for a file. Browsers leave it empty for some types (e.g. .md). */
export function contentTypeOf(file: { name: string; type: string }): string {
  if (file.type) return file.type;
  const name = file.name.toLowerCase();
  if (name.endsWith(".md") || name.endsWith(".markdown")) return "text/markdown";
  if (name.endsWith(".pdf")) return "application/pdf";
  if (name.endsWith(".docx")) return "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
  if (name.endsWith(".pptx")) return "application/vnd.openxmlformats-officedocument.presentationml.presentation";
  if (name.endsWith(".xlsx")) return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
  return "application/octet-stream";
}

/** GETs a path under the API and returns its bytes, with the session token when there is one. */
async function fetchBytes(path: string): Promise<ArrayBuffer> {
  const headers: Record<string, string> = {};
  const token = getToken();
  if (token) headers.authorization = `Bearer ${token}`;
  const response = await fetch(apiUrl(path), { headers });
  if (!response.ok) {
    const data = await response.json().catch(() => null);
    throw new ApiError(response.status, data?.error?.code ?? "error", data?.error?.message ?? response.statusText);
  }
  return response.arrayBuffer();
}

export const attachments = {
  create: (ws: string, body: { filename: string; content_type: string; size_bytes: number; project_id?: string }) =>
    api<Attachment>("POST", `/workspaces/${ws}/attachments`, body),
  /** Files kept with a project. */
  listForProject: (ws: string, projectId: string) =>
    api<Attachment[]>("GET", `/workspaces/${ws}/attachments?project_id=${projectId}`),
  remove: (id: string) => api<void>("DELETE", `/attachments/${id}`),
  /** Project files whose name or text matches the query. */
  searchProjectFiles: (ws: string, q: string) =>
    api<ProjectFileHit[]>("GET", `/workspaces/${ws}/project-files/search?q=${encodeURIComponent(q)}`),
  /** The stored bytes of an attachment, for previewing. */
  download: (id: string): Promise<ArrayBuffer> => fetchBytes(`/attachments/${id}/content`),
  /** An Office document (pptx, docx, xlsx) converted to PDF by the gateway, for previewing. */
  previewPdf: (id: string): Promise<ArrayBuffer> => fetchBytes(`/attachments/${id}/preview`),
  upload: async (id: string, file: File) => {
    const headers: Record<string, string> = { "content-type": contentTypeOf(file) };
    const token = getToken();
    if (token) headers.authorization = `Bearer ${token}`;
    const response = await fetch(apiUrl(`/attachments/${id}/content`), { method: "PUT", headers, body: file });
    if (!response.ok) {
      const data = await response.json().catch(() => null);
      throw new ApiError(response.status, data?.error?.code ?? "error", data?.error?.message ?? response.statusText);
    }
  },
};

export const connectors = {
  list: (ws: string) => api<Connector[]>("GET", `/workspaces/${ws}/connectors`),
  create: (ws: string, body: { name: string; url: string; enabled?: boolean }) =>
    api<Connector>("POST", `/workspaces/${ws}/connectors`, body),
  update: (id: string, body: { name?: string; url?: string; enabled?: boolean }) =>
    api<Connector>("PATCH", `/connectors/${id}`, body),
  remove: (id: string) => api<void>("DELETE", `/connectors/${id}`),
  /** Starts OAuth sign-in; returns the provider's page to open. */
  signIn: (id: string) => api<{ authorization_url: string }>("POST", `/connectors/${id}/sign-in`),
};

/** The signed-in user's own token usage over the last `days` days. */
export const usage = {
  mine: (days: number) => {
    const to = new Date();
    const from = new Date(to.getTime() - days * 24 * 60 * 60 * 1000);
    const query = `from=${encodeURIComponent(from.toISOString())}&to=${encodeURIComponent(to.toISOString())}`;
    return api<UsageReport>("GET", `/me/usage?${query}`);
  },
};
