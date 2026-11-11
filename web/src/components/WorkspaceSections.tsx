import { useEffect, useState } from "react";
import { audit as auditApi, members as memberApi, workspaceSettings as workspaceApi } from "../api/resources";
import type { AuditEntry, Member, User, Workspace } from "../api/types";

/** Workspace name and deletion. Only the owner can delete the workspace. */
export function WorkspaceSection({ workspace, onError }: { workspace: Workspace; onError: (message: string) => void }) {
  const [name, setName] = useState(workspace.name);
  const [busy, setBusy] = useState(false);
  const isOwner = workspace.role === "owner";
  const isAdmin = isOwner || workspace.role === "admin";

  const save = async () => {
    setBusy(true);
    try {
      await workspaceApi.update(workspace.id, { name: name.trim() });
      // The workspace name is read once when the app starts, so reload to show it everywhere.
      window.location.reload();
    } catch (err) {
      setBusy(false);
      onError(err instanceof Error ? err.message : "Could not save the workspace.");
    }
  };

  const remove = async () => {
    if (!window.confirm(`Delete the workspace “${workspace.name}”? Its chats, projects and files will be removed.`)) return;
    try {
      await workspaceApi.remove(workspace.id);
      window.location.reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not delete the workspace.");
    }
  };

  return (
    <>
      <h3 className="settings-title">Workspace</h3>
      <div className="settings-row">
        <div className="settings-grow">
          <div className="detail-label">Name</div>
          <form
            className="memory-add"
            onSubmit={(event) => {
              event.preventDefault();
              if (name.trim()) void save();
            }}
          >
            <input value={name} onChange={(e) => setName(e.target.value)} maxLength={100} disabled={!isAdmin} aria-label="Workspace name" />
            <button className="primary" type="submit" disabled={!isAdmin || busy || !name.trim() || name.trim() === workspace.name}>
              Save
            </button>
          </form>
          <div className="muted small">Address: {workspace.slug}</div>
        </div>
      </div>
      {isOwner && (
        <div className="settings-row">
          <div>
            <div className="detail-label">Delete workspace</div>
            <div className="muted small">Removes every chat, project and file in it. This cannot be undone.</div>
          </div>
          <button onClick={() => void remove()}>Delete</button>
        </div>
      )}
    </>
  );
}

/** Member list, roles, adding and removing. Owners change roles; owners and admins remove members. */
export function MembersSection({ workspace, user, onError }: { workspace: Workspace; user: User; onError: (message: string) => void }) {
  const [list, setList] = useState<Member[] | null>(null);
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<"admin" | "member">("member");
  const [busy, setBusy] = useState(false);
  const isOwner = workspace.role === "owner";

  const reload = () =>
    memberApi
      .list(workspace.id)
      .then(setList)
      .catch((err: unknown) => onError(err instanceof Error ? err.message : "Could not load members."));

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspace.id]);

  const canChangeRole = (member: Member) => isOwner && member.role !== "owner";
  const canRemove = (member: Member) =>
    member.role !== "owner" && member.user_id !== user.id && (isOwner || (workspace.role === "admin" && member.role === "member"));

  const add = async () => {
    setBusy(true);
    try {
      await memberApi.add(workspace.id, { email: email.trim(), role });
      setEmail("");
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not add the member.");
    } finally {
      setBusy(false);
    }
  };

  const changeRole = async (member: Member, next: "admin" | "member") => {
    try {
      await memberApi.changeRole(workspace.id, member.user_id, next);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not change the role.");
    }
  };

  const remove = async (member: Member) => {
    if (!window.confirm(`Remove ${member.email} from this workspace?`)) return;
    try {
      await memberApi.remove(workspace.id, member.user_id);
      await reload();
    } catch (err) {
      onError(err instanceof Error ? err.message : "Could not remove the member.");
    }
  };

  return (
    <>
      <h3 className="settings-title">Members</h3>
      <div className="settings-row settings-row-top">
        <div className="settings-grow">
          <div className="muted small">
            Add someone by the email address of their account. They must have signed up first.
          </div>
          <form
            className="memory-add"
            onSubmit={(event) => {
              event.preventDefault();
              if (email.trim()) void add();
            }}
          >
            <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="name@example.com" aria-label="Member email" />
            <select value={role} onChange={(e) => setRole(e.target.value as "admin" | "member")} aria-label="Role">
              <option value="member">Member</option>
              <option value="admin">Admin</option>
            </select>
            <button className="primary" type="submit" disabled={busy || !email.trim()}>
              Add
            </button>
          </form>

          <div className="member-list">
            {list === null && <p className="muted small">Loading…</p>}
            {list?.map((member) => (
              <div key={member.user_id} className="member-row">
                <div className="member-who">
                  <span>{member.display_name || member.email}</span>
                  <span className="muted small">{member.email}</span>
                </div>
                {canChangeRole(member) ? (
                  <select value={member.role} onChange={(e) => void changeRole(member, e.target.value as "admin" | "member")} aria-label={`Role of ${member.email}`}>
                    <option value="member">Member</option>
                    <option value="admin">Admin</option>
                  </select>
                ) : (
                  <span className="muted small">{member.role === "owner" ? "Owner" : member.role === "admin" ? "Admin" : "Member"}</span>
                )}
                {canRemove(member) && <button onClick={() => void remove(member)}>Remove</button>}
              </div>
            ))}
          </div>
        </div>
      </div>
    </>
  );
}

/** The workspace audit log, newest first. Admins and owners can read it. */
export function AuditSection({ workspace, onError }: { workspace: Workspace; onError: (message: string) => void }) {
  const [entries, setEntries] = useState<AuditEntry[]>([]);
  const [members, setMembers] = useState<Member[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [more, setMore] = useState(false);
  const pageSize = 50;

  const load = (offset: number) =>
    auditApi
      .list(workspace.id, offset, pageSize)
      .then((page) => {
        setEntries((prev) => (offset === 0 ? page : [...prev, ...page]));
        setMore(page.length === pageSize);
        setLoaded(true);
      })
      .catch((err: unknown) => {
        setLoaded(true);
        onError(err instanceof Error ? err.message : "Could not load the audit log.");
      });

  useEffect(() => {
    void load(0);
    memberApi.list(workspace.id).then(setMembers).catch(() => setMembers([]));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspace.id]);

  const who = (actorId: string | null) => {
    if (!actorId) return "System";
    return members.find((m) => m.user_id === actorId)?.email ?? "Removed user";
  };

  return (
    <>
      <h3 className="settings-title">Audit log</h3>
      <div className="settings-row settings-row-top">
        <div className="settings-grow">
          {loaded && entries.length === 0 && <p className="muted small">No activity yet.</p>}
          <div className="audit-list">
            {entries.map((entry) => (
              <div key={entry.id} className="audit-row">
                <span className="muted small">{new Date(entry.created_at).toLocaleString()}</span>
                <span className="audit-action">{entry.action}</span>
                <span className="muted small">{who(entry.actor_id)}</span>
                <span className="muted small audit-target">
                  {entry.target_type} {entry.target_id.slice(0, 8)}
                </span>
              </div>
            ))}
          </div>
          {more && (
            <div>
              <button onClick={() => void load(entries.length)}>Load more</button>
            </div>
          )}
        </div>
      </div>
    </>
  );
}
