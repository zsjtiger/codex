import { useCallback, useEffect, useRef, useState } from "react";
import { api, ApiError, loadStoredToken, setToken } from "../api/client";
import { attachments as attachmentApi, interruptTurn, threads as threadApi } from "../api/resources";
import type { Environment, LoginResponse, Thread, Turn, User, Workspace } from "../api/types";
import { type Frame, GatewaySocket, type SocketStatus } from "../realtime/socket";
import { type ToolUse, mergeToolUses, toolUseFromItem } from "./toolUses";

export interface SendOptions {
  model?: string;
  effort?: string;
  attachment_ids?: string[];
}

export interface Approval {
  requestId: string;
  threadId: string;
  method: string;
  params: Record<string, unknown>;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * All gateway state for the app: the signed-in user, their workspace, threads and
 * turns, the live reply text streamed for each turn, and pending approvals.
 */
export function useCodex() {
  const [user, setUser] = useState<User | null>(null);
  const [checking, setChecking] = useState(true);
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [environment, setEnvironment] = useState<Environment | null>(null);
  const [threads, setThreads] = useState<Thread[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [turns, setTurns] = useState<Turn[]>([]);
  const [agentText, setAgentText] = useState<Record<string, string>>({});
  const [toolUses, setToolUses] = useState<Record<string, ToolUse[]>>({});
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const [socketStatus, setSocketStatus] = useState<SocketStatus>("closed");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [draftProjectId, setDraftProjectId] = useState<string | null>(null);

  // Refs let long-lived callbacks (the WebSocket handler) read current values.
  const activeRef = useRef<string | null>(null);
  const workspaceRef = useRef<Workspace | null>(null);
  const socketRef = useRef<GatewaySocket | null>(null);
  const startedRef = useRef(false);

  const fail = useCallback((err: unknown) => setError(errorMessage(err)), []);

  const refreshThreads = useCallback(async (ws: Workspace) => {
    setThreads(await api<Thread[]>("GET", `/workspaces/${ws.id}/threads?archived=false&limit=100`));
  }, []);

  const loadTurns = useCallback(async (threadId: string) => {
    const [list, uses] = await Promise.all([
      api<Turn[]>("GET", `/threads/${threadId}/turns?limit=100`),
      threadApi.toolUses(threadId),
    ]);
    if (activeRef.current === threadId) setTurns(list);
    const stored = uses.map((use) => ({ id: use.item_id, kind: use.kind, label: use.label }));
    setToolUses((prev) => ({ ...prev, [threadId]: mergeToolUses(stored, prev[threadId] ?? []) }));
  }, []);

  const handleFrame = useCallback(
    (frame: Frame) => {
      switch (frame.type) {
        case "event": {
          const method = frame.method as string;
          const params = frame.params as Record<string, any>;
          if (method === "item/completed" && params.item?.type === "agentMessage") {
            const key = params.turnId as string;
            const text = params.item.text as string;
            setAgentText((prev) => ({ ...prev, [key]: prev[key] ? `${prev[key]}\n\n${text}` : text }));
          }
          const use = method === "item/completed" ? toolUseFromItem(params.item) : null;
          const threadId = (params.threadId ?? frame.threadId) as string | undefined;
          if (use && threadId) {
            setToolUses((prev) => {
              const list = prev[threadId] ?? [];
              if (list.some((existing) => existing.id === use.id)) return prev;
              return { ...prev, [threadId]: [...list, use] };
            });
          }
          if (method === "turn/started" || method === "turn/completed") {
            if (frame.threadId === activeRef.current) void loadTurns(frame.threadId as string).catch(fail);
            if (workspaceRef.current) void refreshThreads(workspaceRef.current).catch(fail);
          }
          break;
        }
        case "approvalRequest":
          setApprovals((prev) => [
            ...prev,
            {
              requestId: frame.requestId as string,
              threadId: frame.threadId as string,
              method: frame.method as string,
              params: frame.params as Record<string, unknown>,
            },
          ]);
          break;
        case "approvalAccepted":
          setApprovals((prev) => prev.filter((a) => a.requestId !== frame.requestId));
          break;
        case "error":
          setError(frame.message as string);
          break;
      }
    },
    [fail, loadTurns, refreshThreads],
  );

  const bootstrap = useCallback(async () => {
    const me = await api<User>("GET", "/me");
    setUser(me);

    const workspaces = await api<Workspace[]>("GET", "/workspaces");
    const ws =
      workspaces[0] ??
      (await api<Workspace>("POST", "/workspaces", { slug: `personal-${me.id.slice(0, 8)}`, name: "Personal" }));
    const environments = await api<Environment[]>("GET", `/workspaces/${ws.id}/environments`);
    const env =
      environments[0] ?? (await api<Environment>("POST", `/workspaces/${ws.id}/environments`, { name: "local" }));

    workspaceRef.current = ws;
    setWorkspace(ws);
    setEnvironment(env);
    await refreshThreads(ws);

    if (!socketRef.current) {
      socketRef.current = new GatewaySocket(handleFrame, setSocketStatus);
      await socketRef.current.start();
    }
    if (activeRef.current) socketRef.current.subscribe(activeRef.current);
  }, [handleFrame, refreshThreads]);

  // Restore a stored session once. The socket then lives for the page's lifetime.
  useEffect(() => {
    if (startedRef.current) return;
    startedRef.current = true;
    const stored = loadStoredToken();
    if (!stored) {
      setChecking(false);
      return;
    }
    setToken(stored);
    bootstrap()
      .catch((err) => {
        if (err instanceof ApiError && err.status === 401) setToken(null);
        else fail(err);
      })
      .finally(() => setChecking(false));
  }, [bootstrap, fail]);

  const signIn = useCallback(
    async (email: string) => {
      setError(null);
      const session = await api<LoginResponse>("POST", "/auth/sessions", { email });
      setToken(session.token);
      await bootstrap();
    },
    [bootstrap],
  );

  const signOut = useCallback(async () => {
    await api("DELETE", "/auth/sessions").catch(() => undefined);
    socketRef.current?.stop();
    socketRef.current = null;
    setToken(null);
    workspaceRef.current = null;
    activeRef.current = null;
    setUser(null);
    setWorkspace(null);
    setEnvironment(null);
    setThreads([]);
    setActiveId(null);
    setTurns([]);
    setAgentText({});
    setApprovals([]);
  }, []);

  const selectThread = useCallback(
    async (threadId: string | null) => {
      const previous = activeRef.current;
      activeRef.current = threadId;
      setActiveId(threadId);
      setTurns([]);
      if (previous && previous !== threadId) socketRef.current?.unsubscribe(previous);
      if (threadId) {
        socketRef.current?.subscribe(threadId);
        await loadTurns(threadId).catch(fail);
      }
    },
    [fail, loadTurns],
  );

  const sendMessage = useCallback(
    async (text: string, options: SendOptions = {}) => {
      const input = text.trim();
      const ws = workspaceRef.current;
      if (!input || sending || !ws || !environment) return;
      setSending(true);
      setError(null);
      try {
        let threadId = activeRef.current;
        if (!threadId) {
          const created = await threadApi.create(ws.id, {
            title: input.slice(0, 60),
            environment_id: environment.id,
            project_id: draftProjectId,
          });
          threadId = created.id;
          await refreshThreads(ws);
          await selectThread(threadId);
        }
        await threadApi.send(threadId, { input, ...options });
        await loadTurns(threadId);
        await refreshThreads(ws);
      } catch (err) {
        fail(err);
      } finally {
        setSending(false);
      }
    },
    [draftProjectId, environment, fail, loadTurns, refreshThreads, selectThread, sending],
  );

  const interrupt = useCallback(
    async (turnId: string) => {
      try {
        await interruptTurn(turnId);
      } catch (err) {
        fail(err);
      }
    },
    [fail],
  );

  const uploadAttachment = useCallback(
    async (file: File): Promise<string> => {
      const ws = workspaceRef.current;
      if (!ws) throw new Error("No workspace yet.");
      const meta = await attachmentApi.create(ws.id, {
        filename: file.name,
        content_type: file.type || "application/octet-stream",
        size_bytes: file.size,
      });
      await attachmentApi.upload(meta.id, file);
      return meta.id;
    },
    [],
  );

  const answerApproval = useCallback((approval: Approval, result: unknown) => {
    socketRef.current?.answerApproval(approval.requestId, result);
    setApprovals((prev) => prev.filter((a) => a.requestId !== approval.requestId));
  }, []);

  return {
    checking,
    user,
    workspace,
    environment,
    threads,
    activeId,
    turns,
    agentText,
    toolUses,
    approvals,
    socketStatus,
    error,
    sending,
    dismissError: () => setError(null),
    draftProjectId,
    setDraftProjectId,
    refreshThreads: () => (workspaceRef.current ? refreshThreads(workspaceRef.current) : Promise.resolve()),
    interrupt,
    uploadAttachment,
    reportError: fail,
    signIn,
    signOut,
    selectThread,
    newChat: () => void selectThread(null),
    sendMessage,
    answerApproval,
  };
}

export type CodexState = ReturnType<typeof useCodex>;
