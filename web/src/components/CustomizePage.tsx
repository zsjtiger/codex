import { useEffect, useState } from "react";
import { connectors as connectorApi, skills as skillApi } from "../api/resources";
import type { Connector, Skill } from "../api/types";

const sections = ["Skills", "Connectors", "Plugins"] as const;
type Section = (typeof sections)[number];

interface CustomizePageProps {
  workspaceId: string;
  isAdmin: boolean;
  onBack: () => void;
  onError: (message: string) => void;
}

function connectorStatusLabel(c: Connector): string {
  if (!c.enabled) return "Disabled";
  switch (c.status) {
    case "ready":
      return "Connected";
    case "starting":
      return "Connecting…";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    default:
      return "Not checked";
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
      fail(err, "Could not load customizations.");
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
      fail(err, `Could not add the ${section === "Connectors" ? "connector" : "item"}.`);
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

  const addLabel = section === "Connectors" ? "Add connector" : section === "Plugins" ? "Add plugin" : "Add skill";
  const formValid = section === "Connectors" ? name.trim() && url.trim() : name.trim() && instructions.trim();

  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>
          ← Back to chat
        </button>
        <h1>Customize</h1>

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
                {name}
              </button>
            ))}
          </div>
          {isAdmin && !creating && (
            <button className="add-button" onClick={() => setCreating(true)}>
              {addLabel}
            </button>
          )}
        </div>

        {!isAdmin && <p className="muted small">Only workspace admins can add or change these.</p>}

        {creating && (
          <form
            className="form-card"
            onSubmit={(event) => {
              event.preventDefault();
              void create();
            }}
          >
            <label className="field">
              <span>Name</span>
              <input value={name} onChange={(e) => setName(e.target.value)} maxLength={100} required />
            </label>
            {section === "Connectors" ? (
              <label className="field">
                <span>Server URL (http or https)</span>
                <input value={url} onChange={(e) => setUrl(e.target.value)} maxLength={2000} placeholder="https://example.com/mcp" required />
              </label>
            ) : (
              <>
                <label className="field">
                  <span>Description</span>
                  <input value={description} onChange={(e) => setDescription(e.target.value)} maxLength={500} />
                </label>
                <label className="field">
                  <span>Instructions</span>
                  <textarea value={instructions} onChange={(e) => setInstructions(e.target.value)} rows={6} maxLength={20000} required />
                </label>
              </>
            )}
            <div className="form-actions">
              <button type="button" onClick={resetForm}>
                Cancel
              </button>
              <button className="primary" type="submit" disabled={!formValid}>
                {addLabel}
              </button>
            </div>
          </form>
        )}

        {section === "Connectors" ? (
          <>
            {connectorItems.length === 0 && !creating && <p className="empty-state">No connectors yet.</p>}
            <div className="list">
              {connectorItems.map((c) => (
                <div key={c.id} className="list-row">
                  <div className="list-main">
                    <div className="list-title">{c.name}</div>
                    <div className="list-sub">
                      Web · <code>{c.key}</code> · {c.url}
                    </div>
                  </div>
                  <div className="list-actions">
                    <span className={`status ${c.enabled && c.status === "ready" ? "status-on" : ""}`} title={c.status_error ?? undefined}>
                      {connectorStatusLabel(c)}
                    </span>
                    {isAdmin && (
                      <button onClick={() => void act(() => connectorApi.update(c.id, { enabled: !c.enabled }), "Could not update the connector.")}>
                        {c.enabled ? "Disable" : "Enable"}
                      </button>
                    )}
                    {isAdmin && <button onClick={() => void act(() => connectorApi.remove(c.id), "Could not remove the connector.")}>Remove</button>}
                  </div>
                </div>
              ))}
            </div>
            <p className="muted small">
              Enabled connectors are added to chats that start after the change. Status is the last report from codex in any chat; it shows "Not checked" until a chat has started with the connector. Connectors that need sign-in are not supported yet.
            </p>
          </>
        ) : (
          (() => {
            const items = section === "Plugins" ? pluginItems : skillItems;
            return (
              <>
                {items.length === 0 && !creating && (
                  <p className="empty-state">{section === "Plugins" ? "No plugins yet." : "No skills added yet."}</p>
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
                          <button onClick={() => void act(() => skillApi.update(skill.id, { enabled: !skill.enabled }), "Could not update the item.")}>
                            {skill.enabled ? "Turn off" : "Turn on"}
                          </button>
                        )}
                        {isAdmin && <button onClick={() => void act(() => skillApi.remove(skill.id), "Could not delete the item.")}>Delete</button>}
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
