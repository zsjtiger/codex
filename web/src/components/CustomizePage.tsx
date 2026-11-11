import { useEffect, useState } from "react";
import { connectors as connectorApi, skills as skillApi } from "../api/resources";
import type { Connector, Skill } from "../api/types";
import { tx } from "../i18n";

const sections = ["Skills", "Connectors", "Plugins"] as const;
type Section = (typeof sections)[number];

interface CustomizePageProps {
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

/**
 * Skills and plugins are instruction bundles added to new chats. Connectors are MCP servers
 * that codex can call from new chats. Only admins change any of them.
 */
export function CustomizePage({ workspaceId, isAdmin, onBack, onError }: CustomizePageProps) {
  const [section, setSection] = useState<Section>("Skills");
  const [skillItems, setSkillItems] = useState<Skill[]>([]);
  const [pluginItems, setPluginItems] = useState<Skill[]>([]);
  const [connectorItems, setConnectorItems] = useState<Connector[]>([]);
  const [creating, setCreating] = useState(false);
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
      setSkillItems(skills);
      setPluginItems(plugins);
      setConnectorItems(connectors);
    } catch (err) {
      fail(err, tx("Could not load customizations."));
    }
  };

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  const resetForm = () => {
    setCreating(false);
    setName("");
    setDescription("");
    setInstructions("");
    setUrl("");
  };

  const create = async () => {
    try {
      if (section === "Connectors") {
        await connectorApi.create(workspaceId, { name: name.trim(), url: url.trim() });
      } else {
        await skillApi.create(workspaceId, {
          kind: section === "Plugins" ? "plugin" : "skill",
          name: name.trim(),
          description: description.trim(),
          instructions: instructions.trim(),
        });
      }
      resetForm();
      await reload();
    } catch (err) {
      fail(err, `${tx("Could not add the")} ${section === "Connectors" ? tx("connector") : tx("item")}.`);
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

  const addLabel = tx(section === "Connectors" ? "Add connector" : section === "Plugins" ? "Add plugin" : "Add skill");
  const formValid = section === "Connectors" ? name.trim() && url.trim() : name.trim() && instructions.trim();

  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>
          {tx("← Back to chat")}
        </button>
        <h1>{tx("Customize")}</h1>

        <div className="customize-toolbar">
          <div className="tabs" role="tablist">
            {sections.map((name) => (
              <button
                key={name}
                role="tab"
                aria-selected={section === name}
                className={`tab ${section === name ? "tab-active" : ""}`}
                onClick={() => {
                  setSection(name);
                  resetForm();
                }}
              >
                {tx(name)}
              </button>
            ))}
          </div>
          {isAdmin && !creating && (
            <button className="add-button" onClick={() => setCreating(true)}>
              {addLabel}
            </button>
          )}
        </div>

        {!isAdmin && <p className="muted small">{tx("Only workspace admins can add or change these.")}</p>}

        {creating && (
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
            {section === "Connectors" ? (
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
                {addLabel}
              </button>
            </div>
          </form>
        )}

        {section === "Connectors" ? (
          <>
            {connectorItems.length === 0 && !creating && <p className="empty-state">{tx("No connectors yet.")}</p>}
            <div className="list">
              {connectorItems.map((c) => (
                <div key={c.id} className="list-row">
                  <div className="list-main">
                    <div className="list-title">{c.name}</div>
                    <div className="list-sub">
                      {tx("Web ·")} <code>{c.key}</code> · {c.url}
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
            </div>
            <p className="muted small">
              {tx("Connected connectors are added to chats that start after the change. Status is the last report from codex in any chat; it shows \"Not checked\" until a chat has started with the connector. Connectors that need sign-in use OAuth: sign in, then start a new chat. Sign-in needs the gateway's public address.")}
            </p>
          </>
        ) : (
          (() => {
            const items = section === "Plugins" ? pluginItems : skillItems;
            return (
              <>
                {items.length === 0 && !creating && (
                  <p className="empty-state">{section === "Plugins" ? tx("No plugins yet.") : tx("No skills added yet.")}</p>
                )}
                <div className="list">
                  {items.map((skill) => (
                    <div key={skill.id} className="list-row">
                      <div className="list-main">
                        <div className="list-title">{skill.name}</div>
                        <div className="list-sub">{skill.description || skill.instructions.slice(0, 120)}</div>
                      </div>
                      <div className="list-actions">
                        <span className={`status ${skill.enabled ? "status-on" : ""}`}>{skill.enabled ? "On" : "Off"}</span>
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
              </>
            );
          })()
        )}
      </div>
    </main>
  );
}
