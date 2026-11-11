import { useCallback, useEffect, useRef, useState } from "react";
import { invites as invitesApi, projects as projectApi } from "./api/resources";
import type { Artifact, Project, ScheduledTask } from "./api/types";
import { ArtifactsPage } from "./components/ArtifactsPage";
import { CustomizePage } from "./components/CustomizePage";
import { CodePage } from "./components/CodePage";
import { Home } from "./components/Home";
import { PreviewProvider } from "./components/Preview";
import { ProjectsPage } from "./components/ProjectsPage";
import { SettingsDialog } from "./components/SettingsDialog";
import { isDesktop, notify, onMenuAction } from "./desktop";
import { ScheduledPage } from "./components/ScheduledPage";
import { SearchDialog } from "./components/SearchDialog";
import { SignIn } from "./components/SignIn";
import { GatewaySetup, isDesktopApp } from "./components/GatewaySetup";
import { gatewayBase } from "./api/config";
import { type NavLabel, Sidebar, type View } from "./components/Sidebar";
import { SidebarIcon } from "./components/icons";
import { useCodex } from "./state/useCodex";
import { tx } from "./i18n";

const NAV_KEY = "codex.sidebar.hidden";

function loadHidden(): NavLabel[] {
  try {
    return JSON.parse(localStorage.getItem(NAV_KEY) ?? "[]") as NavLabel[];
  } catch {
    return [];
  }
}

export function App() {
  // Open on wide screens; start closed on narrow ones, where it overlays the page.
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(() => window.matchMedia("(min-width: 761px)").matches);
  const [view, setView] = useState<View>("chat");
  const [mode, setMode] = useState<"chat" | "code">("chat");
  // The project the Projects page opens on, set when a search result points into a project.
  const [projectFocus, setProjectFocus] = useState<string | null>(null);
  // Where the Scheduled page opens: a new task for a project, or an existing task.
  const [scheduledFocus, setScheduledFocus] = useState<{ projectId: string | null; taskId: string | null } | null>(null);
  const openProject = (projectId: string) => {
    setProjectFocus(projectId);
    setView("projects");
  };
  const [searchOpen, setSearchOpen] = useState(false);
  const [hidden, setHidden] = useState<NavLabel[]>(loadHidden);
  const [projects, setProjects] = useState<Project[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const codex = useCodex();

  const workspaceId = codex.workspace?.id ?? null;
  const isAdmin = codex.workspace ? codex.workspace.role === "admin" || codex.workspace.role === "owner" : false;
  const userId = codex.user?.id ?? "";

  const reloadProjects = useCallback(async () => {
    if (!workspaceId) return;
    setProjects(await projectApi.list(workspaceId));
  }, [workspaceId]);

  useEffect(() => {
    void reloadProjects().catch((err) => setNotice(err instanceof Error ? err.message : tx("Could not load projects.")));
  }, [reloadProjects]);

  // A sign-in link with an invite: once signed in, join that workspace, then reload into it.
  useEffect(() => {
    const token = new URLSearchParams(window.location.search).get("invite");
    if (!token || !codex.user) return;
    invitesApi
      .accept(token)
      .then(() => {
        window.history.replaceState(null, "", window.location.pathname);
        window.location.reload();
      })
      .catch((err: unknown) => {
        window.history.replaceState(null, "", window.location.pathname);
        onError(err instanceof Error ? err.message : tx("This invite is not valid."));
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [codex.user]);

  // Menu items of the desktop app.
  useEffect(() => {
    if (!isDesktop()) return;
    let stop: (() => void) | undefined;
    void onMenuAction((id) => {
      if (id === "new-chat") {
        setView("chat");
        codex.newChat();
      }
      else if (id === "search") setSearchOpen(true);
      else if (id === "settings") setSettingsOpen(true);
      else if (id === "toggle-sidebar") setSidebarOpen((open) => !open);
    }).then((unlisten) => {
      stop = unlisten;
    });
    return () => stop?.();
  }, []);

  // Desktop: a notification when something needs the user and the window is in the background.
  const approvalCount = codex.approvals.length;
  const seenApprovals = useRef(0);
  useEffect(() => {
    if (approvalCount > seenApprovals.current && isDesktop() && document.hidden) {
      void notify("Baize", tx("Approval needed"));
    }
    seenApprovals.current = approvalCount;
  }, [approvalCount]);

  // Cmd/Ctrl+K opens search from anywhere in the app.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearchOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const changeHidden = (next: NavLabel[]) => {
    setHidden(next);
    try {
      localStorage.setItem(NAV_KEY, JSON.stringify(next));
    } catch {
      // Preferences are optional; the sidebar still works without storage.
    }
  };

  if (isDesktopApp() && !gatewayBase()) return <GatewaySetup />;
  if (codex.checking) return <div className="loading">{tx("Loading…")}</div>;
  if (!codex.user || !workspaceId) return <SignIn onSignIn={codex.signIn} onRegister={codex.register} />;

  const error = notice ?? codex.error;
  const dismiss = () => {
    setNotice(null);
    codex.dismissError();
  };
  const onError = (message: string) => setNotice(message);
  const failWith = (err: unknown) => onError(err instanceof Error ? err.message : tx("Something went wrong."));
  const canEditOwned = (createdBy: string | null) => isAdmin || createdBy === userId;
  const openThread = (threadId: string) => {
    const thread = codex.threads.find((t) => t.id === threadId);
    if (thread) setMode(thread.repo_id ? "code" : "chat");
    setView("chat");
    void codex.selectThread(threadId);
  };
  const newChat = () => {
    if (mode === "code") {
      setView("code");
      codex.newChat();
      return;
    }
    setView("chat");
    codex.newChat();
  };
  // Chat and code are separate: each mode lists only its own sessions.
  const changeMode = (next: "chat" | "code") => {
    setMode(next);
    setView(next === "code" ? "code" : "chat");
    codex.newChat();
  };
  const activeThread = codex.threads.find((t) => t.id === codex.activeId) ?? null;

  return (
    <PreviewProvider>
    <div className="app">
      <Sidebar
        open={sidebarOpen}
        onOpenSettings={() => setSettingsOpen(true)}
        onToggle={() => setSidebarOpen(!sidebarOpen)}
        threads={codex.threads.filter((t) => (mode === "code") === Boolean(t.repo_id))}
        mode={mode}
        onMode={changeMode}
        chatActive={codex.threads.some((t) => !t.repo_id && t.state === "running")}
        activeId={codex.activeId}
        activeView={view}
        onSelect={openThread}
        onRenameThread={(id, title) => codex.renameThread(id, title).catch(failWith)}
        onArchiveThread={(id) => codex.archiveThread(id).catch(failWith)}
        onDeleteThread={(id) => codex.deleteThread(id).catch(failWith)}
        onNew={newChat}
        onSearch={() => setSearchOpen(true)}
        onView={setView}
        hidden={hidden}
        onHiddenChange={changeHidden}
        user={codex.user}
        workspaceName={codex.workspace?.name ?? ""}
        socketStatus={codex.socketStatus}
        onSignOut={() => void codex.signOut()}
      />
      {settingsOpen && <SettingsDialog user={codex.user!} workspace={codex.workspace} threads={codex.threads} onClose={() => setSettingsOpen(false)} onError={onError} />}
      {searchOpen && (
        <SearchDialog
          workspaceId={workspaceId}
          threads={codex.threads}
          onOpenThread={openThread}
          onOpenArtifacts={() => setView("artifacts")}
          onOpenProjects={() => setView("projects")}
          onOpenProject={openProject}
          onNew={newChat}
          onClose={() => setSearchOpen(false)}
          onError={onError}
        />
      )}
      <div className="workspace">
        {isDesktop() && <div className="desktop-drag-strip" data-tauri-drag-region />}
        {!sidebarOpen && (
          <button className="icon-button reopen" onClick={() => setSidebarOpen(true)} aria-label={tx("Open sidebar")}>
            <SidebarIcon />
          </button>
        )}
        {view === "projects" && (
          <ProjectsPage
            workspaceId={workspaceId}
            projects={projects}
            threads={codex.threads}
            reload={reloadProjects}
            canEdit={(p: Project) => canEditOwned(p.created_by)}
            onNewScheduled={(projectId) => {
              setScheduledFocus({ projectId, taskId: null });
              setView("scheduled");
            }}
            onOpenScheduled={(taskId) => {
              setScheduledFocus({ projectId: null, taskId });
              setView("scheduled");
            }}
            onStartChat={(projectId) => {
              codex.setDraftProjectId(projectId);
              newChat();
            }}
            onOpenThread={openThread}
            onBack={() => setView("chat")}
            onError={onError}
            initialProjectId={projectFocus}
          />
        )}
        {view === "artifacts" && (
          <ArtifactsPage
            workspaceId={workspaceId}
            userId={userId}
            projects={projects}
            canEdit={(a: Artifact) => canEditOwned(a.created_by)}
            onBack={() => setView("chat")}
            onError={onError}
          />
        )}
        {view === "code" && (
          <CodePage
            workspaceId={workspaceId}
            isAdmin={isAdmin}
            environmentId={codex.environment?.id ?? null}
            onOpenThread={openThread}
            onError={onError}
          />
        )}
        {view === "customize" && <CustomizePage workspaceId={workspaceId} isAdmin={isAdmin} onBack={() => setView("chat")} onError={onError} />}
        {view === "scheduled" && (
          <ScheduledPage
            workspaceId={workspaceId}
            focus={scheduledFocus}
            projects={projects}
            canEdit={(t: ScheduledTask) => canEditOwned(t.created_by)}
            onOpenThread={openThread}
            onBack={() => setView("chat")}
            onError={onError}
          />
        )}
        {view === "chat" && (
          <Home
            user={codex.user}
            threads={codex.threads}
            onOpenThread={openThread}
            workspaceId={workspaceId}
            thread={activeThread}
            turns={codex.turns}
            agentText={codex.agentText}
            toolUses={codex.toolUses}
            environment={codex.environment}
            activity={codex.activity}
            approvals={codex.approvals}
            sending={codex.sending}
            projects={projects}
            draftProjectId={codex.draftProjectId}
            onDraftProject={codex.setDraftProjectId}
            onCreateProject={async (name: string) => {
              const created = await projectApi.create(workspaceId, { name, instructions: "" });
              await reloadProjects();
              return created.id;
            }}
            onThreadChanged={() => void codex.refreshThreads()}
            onSend={(text, options) => void codex.sendMessage(text, options)}
            onStop={(turnId) => void codex.interrupt(turnId)}
            onAnswer={codex.answerApproval}
            onFeedback={codex.setFeedback}
            onOpenArtifacts={() => setView("artifacts")}
            upload={codex.uploadAttachment}
            error={error}
            onError={onError}
            onDismissError={dismiss}
          />
        )}
      </div>
    </div>
    </PreviewProvider>
  );
}
