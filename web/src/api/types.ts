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
  /** Set for a code session: the repository it works in, and its own branch. */
  repo_id: string | null;
  work_branch: string | null;
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
  /** The assistant's reply, kept on the turn once its messages complete. */
  reply?: string | null;
  /** The user's rating of the reply. */
  feedback?: "up" | "down" | null;
  /** Files sent with the message. */
  attachments?: AttachmentRef[];
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
  /** Sign-in for connectors that use OAuth. Null until a sign-in has been started. */
  oauth_state: "pending" | "signed_in" | "failed" | null;
  oauth_error: string | null;
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

/** One run of a scheduled task: the chat it started, when it started. */
export interface ScheduledRun {
  id: string;
  thread_id: string | null;
  started_at: string;
  title: string | null;
  /** A command or file change was declined because no one was there to approve it. */
  needs_approval: boolean;
}

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

/** An attachment as the conversation shows it. */
export interface AttachmentRef {
  id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
}

export interface Attachment {
  id: string;
  workspace_id: string;
  thread_id: string | null;
  filename: string;
  content_type: string;
  size_bytes: number;
  uploaded: boolean;
  /** Bytes of text read from a project file; null when none was read. */
  read_bytes?: number | null;
}

/** A member of a workspace, as the gateway lists them. */
export interface Member {
  user_id: string;
  email: string;
  display_name: string;
  role: "owner" | "admin" | "member";
  joined_at: string;
}

/** One entry of the workspace audit log. */
export interface AuditEntry {
  id: number;
  actor_id: string | null;
  action: string;
  target_type: string;
  target_id: string;
  metadata: unknown;
  created_at: string;
}

/** A note in the user's saved memory. */
export interface MemoryNote {
  id: string;
  content: string;
  created_at: string;
  updated_at: string;
}

export interface MemoryImportResult {
  added: number;
  duplicates: number;
  skipped: number;
}

export interface MemoryView {
  enabled: boolean;
  /** Whether earlier chats are searched for context in new turns. */
  search_chats: boolean;
  notes: MemoryNote[];
}

/** A note kept for one project. */
export interface ProjectNote {
  id: string;
  content: string;
  created_at: string;
}

/** Progress of a memory extraction from one chat. */
export interface FromChatResult {
  status: "queued" | "running" | "completed" | "failed";
  candidates: string[];
  error: string | null;
}

/** A project file that matched a search: its name, the project, and the text around the match. */
export interface ProjectFileHit {
  id: string;
  filename: string;
  project_id: string;
  project_name: string;
  snippet: string | null;
}

export interface SearchResults {
  threads: { id: string; title: string; last_activity_at: string }[];
  turns: { id: string; thread_id: string; thread_title: string; input: string }[];
  artifacts: { id: string; title: string; kind: ArtifactKind }[];
  projects: { id: string; name: string }[];
}

/** Token totals for a time window. The server returns counts in the same shape for totals and days. */
export interface UsageTotals {
  input_tokens: number;
  output_tokens: number;
}

export interface UsageDay extends UsageTotals {
  day: string;
}

export interface UsageReport {
  from: string;
  to: string;
  totals: UsageTotals;
  daily: UsageDay[];
}

/** A GitHub repository linked to the workspace for code sessions. The token is never sent to the browser. */
export interface CodeRepo {
  id: string;
  full_name: string;
  default_branch: string;
  has_token: boolean;
  created_at: string;
  last_synced_at: string | null;
}

/** A repository the connected GitHub account can reach. */
export interface GitHubRepo {
  full_name: string;
  default_branch: string;
  private: boolean;
}

export interface CommitResult {
  sha: string;
  branch: string;
}
