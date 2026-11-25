import { useEffect, useRef, useState } from "react";
import { connectors as connectorApi, marketplace as marketplaceApi, plugins as pluginApi, skills as skillApi } from "../api/resources";
import type { CatalogPlugin, Connector, Plugin, PluginManifest, Skill } from "../api/types";
import { parseManifest } from "../state/pluginDraft";
import { PluginDetail } from "./PluginDetail";
import { tx } from "../i18n";

const filters = ["All", "Connectors", "Skills"] as const;
type Filter = (typeof filters)[number];

/** What the add form creates: a connector (an MCP server), a skill (an instruction bundle), or a plugin. */
type FormKind = "connector" | "skill" | "plugin";

const PLUGIN_EXAMPLE = `{
  "skills": [{ "name": "论文写作", "description": "写章节", "instructions": "使用学术语气。" }],
  "connectors": [{ "name": "alphaXiv", "url": "https://api.alphaxiv.org/mcp" }]
}`;

interface PluginsPageProps {
  workspaceId: string;
  isAdmin: boolean;
  onBack: () => void;
  onError: (message: string) => void;
  /** Opens a new chat that designs a plugin with Baize. */
  onCreateWithBaize: () => void;
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
 * Plugins are bundles of connectors and skills. Installing a plugin adds its skills and connectors to
 * the workspace; uninstalling takes them out again. Connectors are MCP servers codex can call from new
 * chats; skills are instruction bundles added to new chats. Only admins change any of them.
 */
export function PluginsPage({ workspaceId, isAdmin, onBack, onError, onCreateWithBaize }: PluginsPageProps) {
  const [filter, setFilter] = useState<Filter>("All");
  const [query, setQuery] = useState("");
  const [pluginItems, setPluginItems] = useState<Plugin[]>([]);
  const [skillItems, setSkillItems] = useState<Skill[]>([]);
  const [connectorItems, setConnectorItems] = useState<Connector[]>([]);
  const [formKind, setFormKind] = useState<FormKind | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [instructions, setInstructions] = useState("");
  const [url, setUrl] = useState("");
  const [manifestText, setManifestText] = useState("");
  const [catalog, setCatalog] = useState<CatalogPlugin[]>([]);
  const [showAllMarket, setShowAllMarket] = useState(false);
  // The plugin whose detail page is open, if any.
  const [openPluginId, setOpenPluginId] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  // Errors show on this page: the app's notice is only drawn on the chat page.
  const [failure, setFailure] = useState<string | null>(null);
  const report = (message: string) => {
    setFailure(message);
    onError(message);
  };
  const fail = (err: unknown, fallback: string) => report(err instanceof Error ? err.message : fallback);

  const reload = async () => {
    try {
      const [plugins, skills, connectors, market] = await Promise.all([
        pluginApi.list(workspaceId),
        skillApi.list(workspaceId, "skill"),
        connectorApi.list(workspaceId),
        marketplaceApi.list(),
      ]);
      setPluginItems(plugins);
      setCatalog(market);
      setSkillItems(skills);
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
    setManifestText("");
  };

  const openForm = (kind: FormKind) => {
    resetForm();
    setFormKind(kind);
    if (kind === "plugin") setManifestText(PLUGIN_EXAMPLE);
  };

  // An uploaded plugin file fills the form, so its contents can be read before it is added.
  const readUpload = async (file: File) => {
    try {
      const text = await file.text();
      const manifest = parseManifest(text);
      const value = JSON.parse(text) as { name?: unknown; description?: unknown };
      resetForm();
      setFormKind("plugin");
      setName(typeof value.name === "string" ? value.name : file.name.replace(/\.json$/i, ""));
      setDescription(typeof value.description === "string" ? value.description : "");
      setManifestText(JSON.stringify(manifest, null, 2));
    } catch {
      report(tx("The plugin file is not valid JSON."));
    }
  };

  // Adds a marketplace entry to the workspace (unless it is there already) and installs it.
  const installFromCatalog = async (entry: CatalogPlugin) => {
    try {
      const existing = pluginItems.find((p) => p.catalog_key === entry.key);
      const plugin = existing ?? (await marketplaceApi.add(workspaceId, entry.key));
      await pluginApi.install(plugin.id);
      await reload();
    } catch (err) {
      fail(err, tx("Could not install the plugin."));
    }
  };

  const create = async () => {
    try {
      if (formKind === "connector") {
        await connectorApi.create(workspaceId, { name: name.trim(), url: url.trim() });
      } else if (formKind === "plugin") {
        let manifest: PluginManifest;
        try {
          manifest = parseManifest(manifestText);
        } catch {
          report(tx("The plugin is not valid JSON."));
          return;
        }
        await pluginApi.create(workspaceId, { name: name.trim(), description: description.trim(), manifest });
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
      fail(
        err,
        formKind === "connector"
          ? tx("Could not add the connector.")
          : formKind === "plugin"
            ? tx("Could not add the plugin.")
            : tx("Could not add the skill."),
      );
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

  const removePlugin = (plugin: Plugin) => {
    if (plugin.installed && !window.confirm(tx("Delete this plugin? Its installed skills and connectors are removed too."))) return;
    void act(() => pluginApi.remove(plugin.id), tx("Could not delete the plugin."));
  };

  const needle = query.trim().toLowerCase();
  const matches = (...fields: string[]) => !needle || fields.some((field) => field.toLowerCase().includes(needle));
  const showPlugins = filter === "All";
  const showConnectors = filter !== "Skills";
  const showSkills = filter !== "Connectors";
  const visiblePlugins = showPlugins ? pluginItems.filter((p) => matches(p.name, p.description)) : [];
  const visibleConnectors = showConnectors ? connectorItems.filter((c) => matches(c.name, c.url, c.key)) : [];
  const visibleSkills = showSkills ? skillItems.filter((s) => matches(s.name, s.description)) : [];
  const nothingShown = visiblePlugins.length === 0 && visibleConnectors.length === 0 && visibleSkills.length === 0;

  const formValid =
    formKind === "connector"
      ? name.trim() && url.trim()
      : formKind === "plugin"
        ? name.trim() && manifestText.trim()
        : name.trim() && instructions.trim();
  const showSections = filter === "All" && !needle;

  const pluginRows = visiblePlugins.map((plugin) => (
            <div key={plugin.id} className="list-row plugin-row">
              <span className="plugin-mark">{markOf(plugin.name)}</span>
              <div className="list-main clickable" onClick={() => setOpenPluginId(plugin.id)}>
                <div className="list-title">
                  {plugin.name} <span className="plugin-kind">{tx("Plugin")}</span>
                </div>
                <div className="list-sub">
                  {plugin.description || `${plugin.manifest.skills.length} ${tx("skills")} · ${plugin.manifest.connectors.length} ${tx("connectors")}`}
                </div>
              </div>
              <div className="list-actions">
                <span className={`status ${plugin.installed ? "status-on" : ""}`}>
                  {tx(plugin.installed ? "Installed" : "Not installed")}
                </span>
                {isAdmin &&
                  (plugin.installed ? (
                    <button onClick={() => void act(() => pluginApi.uninstall(plugin.id), tx("Could not uninstall the plugin."))}>{tx("Uninstall")}</button>
                  ) : (
                    <button className="primary" onClick={() => void act(() => pluginApi.install(plugin.id), tx("Could not install the plugin."))}>
                      {tx("Install")}
                    </button>
                  ))}
                {isAdmin && <button onClick={() => removePlugin(plugin)}>{tx("Delete")}</button>}
              </div>
            </div>
          ));
  const connectorRows = visibleConnectors.map((c) => (
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
          ));
  const skillRows = visibleSkills.map((skill) => (
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
          ));

  const openedPlugin = pluginItems.find((p) => p.id === openPluginId) ?? null;
  if (openedPlugin) {
    const deleteOpened = async () => {
      if (openedPlugin.installed && !window.confirm(tx("Delete this plugin? Its installed skills and connectors are removed too."))) return;
      try {
        await pluginApi.remove(openedPlugin.id);
        setOpenPluginId(null);
        await reload();
      } catch (err) {
        fail(err, tx("Could not delete the plugin."));
      }
    };
    return (
      <main className="customize-page">
        <div className="customize-inner">
          {failure && (
            <p className="banner error" role="alert">
              {failure}
              <button className="banner-close" onClick={() => setFailure(null)} aria-label={tx("Dismiss")}>
                ×
              </button>
            </p>
          )}
          <PluginDetail
            plugin={openedPlugin}
            connectors={connectorItems}
            isAdmin={isAdmin}
            onBack={() => setOpenPluginId(null)}
            onInstall={() => void act(() => pluginApi.install(openedPlugin.id), tx("Could not install the plugin."))}
            onUninstall={() => void act(() => pluginApi.uninstall(openedPlugin.id), tx("Could not uninstall the plugin."))}
            onDelete={() => void deleteOpened()}
          />
        </div>
      </main>
    );
  }

  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>
          {tx("← Back to chat")}
        </button>
        <h1>{tx("Plugins")}</h1>

        {failure && (
          <p className="banner error" role="alert">
            {failure}
            <button className="banner-close" onClick={() => setFailure(null)} aria-label={tx("Dismiss")}>
              ×
            </button>
          </p>
        )}

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
                    <button role="menuitem" onClick={() => { setMenuOpen(false); fileRef.current?.click(); }}>
                      {tx("Upload plugin")}
                    </button>
                    <button role="menuitem" onClick={() => { setMenuOpen(false); openForm("plugin"); }}>
                      {tx("Create a plugin")}
                    </button>
                    <button role="menuitem" onClick={() => { setMenuOpen(false); onCreateWithBaize(); }}>
                      {tx("Create with Baize")}
                    </button>
                    <button role="menuitem" onClick={() => { setMenuOpen(false); setShowAllMarket(true); }}>
                      {tx("Browse marketplace")}
                    </button>
                    <button role="menuitem" onClick={() => { setMenuOpen(false); openForm("connector"); }}>
                      {tx("Add custom connector")}
                    </button>
                    <button role="menuitem" onClick={() => { setMenuOpen(false); openForm("skill"); }}>
                      {tx("Create a skill")}
                    </button>
                  </div>
                </>
              )}
              <input
                ref={fileRef}
                type="file"
                accept=".json,application/json"
                hidden
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  event.target.value = "";
                  if (file) void readUpload(file);
                }}
              />
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
            {formKind === "connector" && (
              <label className="field">
                <span>{tx("Server URL (http or https)")}</span>
                <input value={url} onChange={(e) => setUrl(e.target.value)} maxLength={2000} placeholder={tx("https://example.com/mcp")} required />
              </label>
            )}
            {formKind === "skill" && (
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
            {formKind === "plugin" && (
              <>
                <label className="field">
                  <span>{tx("Description")}</span>
                  <input value={description} onChange={(e) => setDescription(e.target.value)} maxLength={500} />
                </label>
                <label className="field">
                  <span>{tx("Skills and connectors (JSON)")}</span>
                  <textarea
                    className="plugin-manifest"
                    value={manifestText}
                    onChange={(e) => setManifestText(e.target.value)}
                    rows={10}
                    spellCheck={false}
                    required
                  />
                </label>
                <p className="muted small">
                  {tx("Installing the plugin adds its skills and connectors to the workspace. Uninstalling takes them out again.")}
                </p>
              </>
            )}
            <div className="form-actions">
              <button type="button" onClick={resetForm}>
                {tx("Cancel")}
              </button>
              <button className="primary" type="submit" disabled={!formValid}>
                {formKind === "connector" ? tx("Add connector") : formKind === "plugin" ? tx("Add plugin") : tx("Add skill")}
              </button>
            </div>
          </form>
        )}

        {nothingShown && !formKind && (
          <p className="empty-state">{needle ? tx("No plugins match your search.") : tx("No plugins yet.")}</p>
        )}

        {showSections ? (
          <>
            <h2 className="plugin-section">{tx("Your plugins")}</h2>
            {visiblePlugins.length === 0 && <p className="muted small">{tx("No plugins added yet. Add one from Most used below, or with the Add menu.")}</p>}
            <div className="list plugin-list">{pluginRows}</div>

            <h2 className="plugin-section">{tx("Most used")}</h2>
            <div className="market-grid">
              {(showAllMarket ? catalog : catalog.slice(0, 4)).map((entry) => {
                const added = pluginItems.find((p) => p.catalog_key === entry.key);
                return (
                  <div key={entry.key} className="market-card">
                    <div className="market-head">
                      <span className="plugin-mark">{markOf(entry.name)}</span>
                      <span className="market-name">{entry.name}</span>
                    </div>
                    <div className="muted market-desc">{entry.description}</div>
                    <div className="muted market-meta">
                      {entry.publisher} · {entry.installs} {tx("installs")}
                    </div>
                    <div className="market-actions">
                      {added?.installed ? (
                        <span className="status status-on">{tx("Installed")}</span>
                      ) : (
                        isAdmin && (
                          <button className="primary" onClick={() => void installFromCatalog(entry)}>
                            {tx("Install")}
                          </button>
                        )
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
            {catalog.length > 4 && (
              <button className="link-button" onClick={() => setShowAllMarket((open) => !open)}>
                {showAllMarket ? tx("Show less") : tx("Show all")}
              </button>
            )}

            <h2 className="plugin-section">{tx("Connectors and skills")}</h2>
            <div className="list plugin-list">
              {connectorRows}
              {skillRows}
            </div>
          </>
        ) : (
          <div className="list plugin-list">
            {pluginRows}
            {connectorRows}
            {skillRows}
          </div>
        )}

        {showConnectors && connectorItems.length > 0 && (
          <p className="muted small">
            {tx("Connected connectors are added to chats that start after the change. Status is the last report from codex in any chat; it shows \"Not checked\" until a chat has started with the connector. Connectors that need sign-in use OAuth: sign in, then start a new chat. Sign-in needs the gateway's public address.")}
          </p>
        )}
      </div>
    </main>
  );
}
