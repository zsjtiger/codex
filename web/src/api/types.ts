// Shapes returned by the gateway's /api/v1 routes.

export interface User {
  id: string;
  email: string;
  display_name: string;
}

export interface Workspace {
  id: string;
  slug: string;
  name: string;
  role: string;
}

export interface Environment {
  id: string;
  workspace_id: string;
  name: string;
  status: string;
  endpoint: string | null;
}

export interface Thread {
  id: string;
  workspace_id: string;
  environment_id: string | null;
  project_id: string | null;
  title: string;
  state: "idle" | "running" | "failed";
  codex_thread_id: string | null;
  archived_at: string | null;
  created_at: string;
  last_activity_at: string;
}

export type TurnStatus = "queued" | "running" | "completed" | "failed" | "interrupted";

/** A tool, search, or file action Codex completed in a thread. */
export interface ToolUseRecord {
  id: string;
  item_id: string;
  kind: string;
  label: string;
  created_at: string;
}

export interface Turn {
  id: string;
  thread_id: string;
  seq: number;
  status: TurnStatus;
  input: string;
  error: string | null;
  codex_turn_id: string | null;
  model: string | null;
  effort: string | null;
  created_at: string;
  completed_at: string | null;
}

export interface LoginResponse {
  token: string;
  expires_at: string;
  user: User;
}

export interface Project {
  id: string;
  workspace_id: string;
  name: string;
  instructions: string;
  created_by: string | null;
  created_at: string;
  updated_at: string;
}

export interface Connector {
  id: string;
  workspace_id: string;
  name: string;
  /** Server key used in codex's configuration, derived from the name. */
  key: string;
  url: string;
  enabled: boolean;
  /** Last startup status codex reported, or null if not checked yet. */
  status: "starting" | "ready" | "failed" | "cancelled" | null;
  status_error: string | null;
  created_at: string;
  updated_at: string;
}

export interface Skill {
  id: string;
  workspace_id: string;
  kind: "skill" | "plugin";
  name: string;
  description: string;
  instructions: string;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export type ArtifactKind = "document" | "slides" | "design" | "dashboard";

export interface Artifact {
  id: string;
  workspace_id: string;
  project_id: string | null;
  thread_id: string | null;
  title: string;
  kind: ArtifactKind;
  content: string;
  created_by: string | null;
  created_at: string;
  updated_at: string;
}

export type Frequency = "hourly" | "daily" | "weekdays" | "weekly";

export interface ScheduledTask {
  id: string;
  workspace_id: string;
  name: string;
  prompt: string;
  frequency: Frequency;
  time_of_day: string;
  weekday: number | null;
  environment_id: string | null;
  project_id: string | null;
  enabled: boolean;
  next_run_at: string;
  last_run_at: string | null;
  last_thread_id: string | null;
  created_by: string | null;
  created_at: string;
  updated_at: string;
}

export interface Attachment {
  id: string;
  workspace_id: string;
  thread_id: string | null;
  filename: string;
  content_type: string;
  size_bytes: number;
  uploaded: boolean;
}

export interface SearchResults {
  threads: { id: string; title: string; last_activity_at: string }[];
  turns: { id: string; thread_id: string; thread_title: string; input: string }[];
  artifacts: { id: string; title: string; kind: ArtifactKind }[];
  projects: { id: string; name: string }[];
}
