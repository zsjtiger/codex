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
  title: string;
  state: "idle" | "running" | "failed";
  codex_thread_id: string | null;
  archived_at: string | null;
  created_at: string;
  last_activity_at: string;
}

export type TurnStatus = "queued" | "running" | "completed" | "failed" | "interrupted";

export interface Turn {
  id: string;
  thread_id: string;
  seq: number;
  status: TurnStatus;
  input: string;
  error: string | null;
  codex_turn_id: string | null;
  created_at: string;
  completed_at: string | null;
}

export interface LoginResponse {
  token: string;
  expires_at: string;
  user: User;
}
