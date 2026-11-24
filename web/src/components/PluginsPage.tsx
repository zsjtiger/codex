import { useEffect, useState } from "react";
import { connectors as connectorApi, skills as skillApi } from "../api/resources";
import type { Connector, Skill } from "../api/types";
import { tx } from "../i18n";

const filters = ["All", "Connectors", "Skills"] as const;
type Filter = (typeof filters)[number];

/** What the add form creates: a connector (an MCP server) or a skill (an instruction bundle). */
type FormKind = "connector" | "skill";

interface PluginsPageProps {
  workspaceId: string;
  isAdmin: boolean;
  onBack: () => void;
  onError: (message: string) => void;
}

function connectorStatusLabel(c: Connector): string {
  if (!c.enabled) return tx("Disabled");
  switch (c.status) {
    case "ready":
      return tx("Connected");
    case "starting":
      return tx("Connecting…");
    case "failed":
      return tx("Failed");
    case "cancelled":
      return tx("Cancelled");
    default:
      return tx("Not checked");
  }
}

/** The first letter of a name, shown in a square as the item's mark. */
function markOf(name: string): string {
  return (name.trim().charAt(0) || "?").toUpperCase();
}

/**
 * Plugins are the connectors and skills a workspace adds to its chats. Connectors are MCP servers
 * codex can call from new chats; skills are instruction bundles added to new chats. Only admins
 * change any of them.
 */
export function PluginsPage({ workspaceId, isAdmin, onBack, onError }: PluginsPageProps) {
  const [filter, setFilter] = useState<Filter>("All");
  const [query, setQuery] = useState("");
  const [skillItems, setSkillItems] = useState<Skill[]>([]);
  const [connectorItems, setConnectorItems] = useState<Connector[]>([]);
  // Skills and plugins are both instruction bundles; the list shows which kind each one is.
  const [formKind, setFormKind] = useState<FormKind | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [instructions, setInstructions] = useState("");
  const [url, setUrl] = useState("");

  const fail = (err: unknown, fallback: string) => onError(err instanceof Error ? err.message : fallback);

  const reload = async () => {
    try {
      const [skills, plugins, connectors] = await Promise.all([
        skillApi.list(workspaceId, "skill"),
        skillApi.list(workspaceId, "plugin"),
        connectorApi.list(workspaceId),
      ]);
      setSkillItems([...skills, ...plugins]);
      setConnectorItems(connectors);
    } catch (err) {
      fail(err, tx("Could not load plugins."));
    }
  };

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  const resetForm = () => {
    setFormKind(null);
    setName("");
    setDescription("");
    setInstructions("");
    setUrl("");
  };

  const openForm = (kind: FormKind) => {
    resetForm();
    setFormKind(kind);
  };

  const create = async () => {
    try {
      if (formKind === "connector") {
        await connectorApi.create(workspaceId, { name: name.trim(), url: url.trim() });
      } else {
        await skillApi.create(workspaceId, {
          kind: "skill",
          name: name.trim(),
          description: description.trim(),
          instructions: instructions.trim(),
        });
      }
      resetForm();
      await reload();
    } catch (err) {
      fail(err, formKind === "connector" ? tx("Could not add the connector.") : tx("Could not add the skill."));
    }
  };

  // Opens the provider's sign-in page, then reloads the connector until the sign-in finishes.
  const signIn = async (connector: Connector) => {
    try {
      const { authorization_url } = await connectorApi.signIn(connector.id);
      window.open(authorization_url, "_blank", "noopener");
      await reload();
      for (let attempt = 0; attempt < 200; attempt++) {
        await new Promise((resolve) => setTimeout(resolve, 3000));
        const latest = await connectorApi.list(workspaceId);
        setConnectorItems(latest);
        if (latest.find((c) => c.id === connector.id)?.oauth_state !== "pending") break;
      }
    } catch (err) {
      fail(err, tx("Could not start the sign-in."));
    }
  };

  const act = async (fn: () => Promise<unknown>, fallback: string) => {
    try {
      await fn();
      await reload();
    } catch (err) {
      fail(err, fallback);
    }
  };

  const needle = query.trim().toLowerCase();
  const matches = (...fields: string[]) => !needle || fields.some((field) => field.toLowerCase().includes(needle));
  const showConnectors = filter !== "Skills";
  const showSkills = filter !== "Connectors";
  const visibleConnectors = showConnectors ? connectorItems.filter((c) => matches(c.name, c.url, c.key)) : [];
  const visibleSkills = showSkills ? skillItems.filter((s) => matches(s.name, s.description)) : [];
  const nothingShown = visibleConnectors.length === 0 && visibleSkills.length === 0;

  const formValid = formKind === "connector" ? name.trim() && url.trim() : name.trim() && instructions.trim();

  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>
          {tx("← Back to chat")}
        </button>
        <h1>{tx("Plugins")}</h1>

        <div className="plugin-toolbar">
          <input
            className="plugin-search"
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={tx("Search plugins")}
            aria-label={tx("Search plugins")}
          />
          <div className="tabs" role="tablist">
            {filters.map((name) => (
              <button
                key={name}
                role="tab"
                aria-selected={filter === name}
                className={`tab ${filter === name ? "tab-active" : ""}`}
                onClick={() => {
                  setFilter(name);
                  resetForm();
                }}
              >
                {tx(name)}
              </button>
            ))}
          </div>
          {isAdmin && !formKind && (
            <div className="plugin-add">
              <button className="add-button" aria-haspopup="menu" aria-expanded={menuOpen} onClick={() => setMenuOpen((open) => !open)}>
                {tx("Add")} ▾
              </button>
              {menuOpen && (
                <>
                  <div className="plugin-menu-backdrop" onClick={() => setMenuOpen(false)} />
                  <div className="plugin-menu" role="menu">
                    <button role="menuitem" onClick={() => { setMenuOpen(false); openForm("connector"); }}>
                      {tx("Add custom connector")}
                    </button>
                    <button role="menuitem" onClick={() => { setMenuOpen(false); openForm("skill"); }}>
                      {tx("Create a skill")}
                    </button>
                  </div>
                </>
              )}
            </div>
          )}
        </div>

        {!isAdmin && <p className="muted small">{tx("Only workspace admins can add or change these.")}</p>}

        {formKind && (
          <form
            className="form-card"
            onSubmit={(event) => {
              event.preventDefault();
              void create();
            }}
          >
            <label className="field">
              <span>{tx("Name")}</span>
              <input value={name} onChange={(e) => setName(e.target.value)} maxLength={100} required />
            </label>
            {formKind === "connector" ? (
              <label className="field">
                <span>{tx("Server URL (http or https)")}</span>
                <input value={url} onChange={(e) => setUrl(e.target.value)} maxLength={2000} placeholder={tx("https://example.com/mcp")} required />
              </label>
            ) : (
              <>
                <label className="field">
                  <span>{tx("Description")}</span>
                  <input value={description} onChange={(e) => setDescription(e.target.value)} maxLength={500} />
                </label>
                <label className="field">
                  <span>{tx("Instructions")}</span>
                  <textarea value={instructions} onChange={(e) => setInstructions(e.target.value)} rows={6} maxLength={20000} required />
                </label>
              </>
            )}
            <div className="form-actions">
              <button type="button" onClick={resetForm}>
                {tx("Cancel")}
              </button>
              <button className="primary" type="submit" disabled={!formValid}>
                {formKind === "connector" ? tx("Add connector") : tx("Add skill")}
              </button>
            </div>
          </form>
        )}

        {nothingShown && !formKind && (
          <p className="empty-state">{needle ? tx("No plugins match your search.") : tx("No plugins yet.")}</p>
        )}

        <div className="list plugin-list">
          {visibleConnectors.map((c) => (
            <div key={c.id} className="list-row plugin-row">
              <span className="plugin-mark">{markOf(c.name)}</span>
              <div className="list-main">
                <div className="list-title">
                  {c.name} <span className="plugin-kind">{tx("Connector")}</span>
                </div>
                <div className="list-sub">
                  <code>{c.key}</code> · {c.url}
                </div>
              </div>
              <div className="list-actions">
                {c.oauth_state && (
                  <span className={`status ${c.oauth_state === "signed_in" ? "status-on" : ""}`} title={c.oauth_error ?? undefined}>
                    {c.oauth_state === "pending" ? tx("Waiting for sign-in…") : c.oauth_state === "signed_in" ? tx("Signed in") : tx("Sign-in failed")}
                  </span>
                )}
                <span className={`status ${c.enabled && c.status === "ready" ? "status-on" : ""}`} title={c.status_error ?? undefined}>
                  {connectorStatusLabel(c)}
                </span>
                {c.enabled && (
                  <button onClick={() => void signIn(c)} disabled={c.oauth_state === "pending"}>
                    {c.oauth_state === "signed_in" ? tx("Sign in again") : tx("Sign in")}
                  </button>
                )}
                {isAdmin && (
                  <button onClick={() => void act(() => connectorApi.update(c.id, { enabled: !c.enabled }), tx("Could not update the connector."))}>
                    {c.enabled ? tx("Disconnect") : tx("Connect")}
                  </button>
                )}
                {isAdmin && <button onClick={() => void act(() => connectorApi.remove(c.id), tx("Could not remove the connector."))}>{tx("Remove")}</button>}
              </div>
            </div>
          ))}
          {visibleSkills.map((skill) => (
            <div key={skill.id} className="list-row plugin-row">
              <span className="plugin-mark">{markOf(skill.name)}</span>
              <div className="list-main">
                <div className="list-title">
                  {skill.name} <span className="plugin-kind">{tx(skill.kind === "plugin" ? "Plugin" : "Skill")}</span>
                </div>
                <div className="list-sub">{skill.description || skill.instructions.slice(0, 120)}</div>
              </div>
              <div className="list-actions">
                <span className={`status ${skill.enabled ? "status-on" : ""}`}>{tx(skill.enabled ? "On" : "Off")}</span>
                {isAdmin && (
                  <button onClick={() => void act(() => skillApi.update(skill.id, { enabled: !skill.enabled }), tx("Could not update the item."))}>
                    {skill.enabled ? tx("Turn off") : tx("Turn on")}
                  </button>
                )}
                {isAdmin && <button onClick={() => void act(() => skillApi.remove(skill.id), tx("Could not delete the item."))}>{tx("Delete")}</button>}
              </div>
            </div>
          ))}
        </div>

        {showConnectors && connectorItems.length > 0 && (
          <p className="muted small">
            {tx("Connected connectors are added to chats that start after the change. Status is the last report from codex in any chat; it shows \"Not checked\" until a chat has started with the connector. Connectors that need sign-in use OAuth: sign in, then start a new chat. Sign-in needs the gateway's public address.")}
          </p>
        )}
      </div>
    </main>
  );
}
