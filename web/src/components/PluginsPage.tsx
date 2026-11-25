import { useEffect, useRef, useState } from "react";
import { connectors as connectorApi, environments, marketplace as marketplaceApi, plugins as pluginApi, skills as skillApi } from "../api/resources";
import type { CatalogPlugin, Connector, McpServer, Plugin, PluginManifest, Skill } from "../api/types";
import { parseManifest } from "../state/pluginDraft";
import { CheckCircleIcon, ChevronDownIcon, PlugIcon, SearchIcon } from "./icons";
import { CodexSkills } from "./CodexSkills";
import { PluginDetail } from "./PluginDetail";
import { RowMenu, type MenuItem } from "./RowMenu";
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
  /** The environment whose codex is asked for the live state of connectors. */
  environmentId: string | null;
  isAdmin: boolean;
  onBack: () => void;
  onError: (message: string) => void;
  /** Opens a new chat that designs a plugin with Baize. */
  onCreateWithBaize: () => void;
}

/** What codex reports for a server right now, as a short line. */
function liveLabel(server: McpServer | undefined, checked: boolean): string {
  if (!checked) return "";
  if (!server) return tx("Not running in codex");
  const tools = server.tool_names.length;
  switch (server.runtime_status) {
    case "connected":
      return `${tx("Running")} · ${tools} ${tx(tools === 1 ? "tool" : "tools")}`;
    case "starting":
      return tx("Starting in codex");
    case "authenticationRequired":
      return tx("Sign-in required in codex");
    case "failed":
      return server.tools_error ? `${tx("Failed")}: ${server.tools_error}` : tx("Failed in codex");
    case "cancelled":
      return tx("Cancelled in codex");
    case "disabled":
      return tx("Disabled in codex");
    default:
      return tx("Not started in codex");
  }
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

/** Names as a short line: the first three, then how many more. */
function namesLine(names: string[]): string {
  if (names.length <= 3) return names.join(", ");
  return `${names.slice(0, 3).join(", ")} ${tx("and")} ${names.length - 3} ${tx("more")}`;
}

/**
 * Plugins are bundles of connectors and skills. Installing a plugin adds its skills and connectors to
 * the workspace; uninstalling takes them out again. Connectors are MCP servers codex can call from new
 * chats; skills are instruction bundles added to new chats. Only admins change any of them.
 */
export function PluginsPage({ workspaceId, environmentId, isAdmin, onBack, onError, onCreateWithBaize }: PluginsPageProps) {
  const [filter, setFilter] = useState<Filter>("All");
  const [query, setQuery] = useState("");
  const [pluginItems, setPluginItems] = useState<Plugin[]>([]);
  const [skillItems, setSkillItems] = useState<Skill[]>([]);
  const [connectorItems, setConnectorItems] = useState<Connector[]>([]);
  // The servers codex has loaded now, by name, from the last "Check now". Null until it has been checked.
  const [live, setLive] = useState<Record<string, McpServer> | null>(null);
  const [checking, setChecking] = useState(false);
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

  // Escape closes the Add menu.
  useEffect(() => {
    if (!menuOpen) return;
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [menuOpen]);

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

  // Each row's "…" menu holds the actions for that item.
  const pluginMenu = (plugin: Plugin): MenuItem[] => {
    if (!isAdmin) return [];
    return [
      plugin.installed
        ? { label: tx("Uninstall"), onClick: () => void act(() => pluginApi.uninstall(plugin.id), tx("Could not uninstall the plugin.")) }
        : { label: tx("Install"), onClick: () => void act(() => pluginApi.install(plugin.id), tx("Could not install the plugin.")) },
      { label: tx("Delete"), danger: true, onClick: () => removePlugin(plugin) },
    ];
  };
  const connectorMenu = (c: Connector): MenuItem[] => {
    const items: MenuItem[] = [];
    if (c.enabled) {
      items.push({
        label: c.oauth_state === "signed_in" ? tx("Sign in again") : tx("Sign in"),
        onClick: () => void signIn(c),
      });
    }
    if (isAdmin) {
      items.push({
        label: c.enabled ? tx("Disconnect") : tx("Connect"),
        onClick: () => void act(() => connectorApi.update(c.id, { enabled: !c.enabled }), tx("Could not update the connector.")),
      });
      items.push({
        label: tx("Remove"),
        danger: true,
        onClick: () => void act(() => connectorApi.remove(c.id), tx("Could not remove the connector.")),
      });
    }
    return items;
  };
  const skillMenu = (skill: Skill): MenuItem[] => {
    if (!isAdmin) return [];
    return [
      {
        label: skill.enabled ? tx("Turn off") : tx("Turn on"),
        onClick: () => void act(() => skillApi.update(skill.id, { enabled: !skill.enabled }), tx("Could not update the item.")),
      },
      { label: tx("Delete"), danger: true, onClick: () => void act(() => skillApi.remove(skill.id), tx("Could not delete the item.")) },
    ];
  };

  const pluginRows = visiblePlugins.map((plugin) => (
    <div key={plugin.id} className="pl-row">
      <span className="pl-icon">
        <PlugIcon />
      </span>
      <div className="pl-main" onClick={() => setOpenPluginId(plugin.id)}>
        <div className="pl-name">{plugin.name}</div>
        <div className="pl-meta">
          {tx("Plugin")} · {plugin.catalog_key ? tx("By Baize") : tx("By you")}
          {!plugin.installed && ` · ${tx("Not installed")}`}
        </div>
      </div>
      <RowMenu items={pluginMenu(plugin)} label={plugin.name} />
    </div>
  ));
  const connectorRows = visibleConnectors.map((c) => (
    <div key={c.id} className="pl-row">
      <span className="pl-icon">
        <PlugIcon />
      </span>
      <div className="pl-main">
        <div className="pl-name">{c.name}</div>
        <div className="pl-meta">
          {tx("Connector")} · {connectorStatusLabel(c)}
          {c.oauth_state === "pending" && ` · ${tx("Waiting for sign-in…")}`}
          {live && <div>{liveLabel(live[c.key], true)}</div>}
        </div>
      </div>
      <RowMenu items={connectorMenu(c)} label={c.name} />
    </div>
  ));
  const skillRows = visibleSkills.map((skill) => (
    <div key={skill.id} className="pl-row">
      <span className="pl-icon">
        <PlugIcon />
      </span>
      <div className="pl-main">
        <div className="pl-name">{skill.name}</div>
        <div className="pl-meta">
          {tx(skill.kind === "plugin" ? "Plugin" : "Skill")} · {tx("By you")} · {tx(skill.enabled ? "On" : "Off")}
        </div>
      </div>
      <RowMenu items={skillMenu(skill)} label={skill.name} />
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

  const addMenuItems: MenuItem[] = [
    { label: tx("Upload plugin"), onClick: () => fileRef.current?.click() },
    { label: tx("Create a plugin"), onClick: () => openForm("plugin") },
    { label: tx("Create with Baize"), onClick: () => onCreateWithBaize() },
    { label: tx("Browse marketplace"), onClick: () => setShowAllMarket(true) },
    { label: tx("Add custom connector"), onClick: () => openForm("connector") },
    { label: tx("Create a skill"), onClick: () => openForm("skill") },
  ];
  const pageMenuItems: MenuItem[] = [
    { label: tx("Browse marketplace"), onClick: () => setShowAllMarket(true) },
    ...(isAdmin ? [{ label: tx("Upload plugin"), onClick: () => fileRef.current?.click() }] : []),
  ];

  return (
    <main className="customize-page">
      <div className="customize-inner">
        <button className="link-button" onClick={onBack}>
          {tx("← Back to chat")}
        </button>
        <div className="pl-title">
          <h1>{tx("Plugins")}</h1>
          <RowMenu items={pageMenuItems} label={tx("More")} />
        </div>

        {failure && (
          <p className="banner error" role="alert">
            {failure}
            <button className="banner-close" onClick={() => setFailure(null)} aria-label={tx("Dismiss")}>
              ×
            </button>
          </p>
        )}

        <div className="pl-toolbar">
          <label className="pl-search">
            <SearchIcon />
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={tx("Search plugins")}
              aria-label={tx("Search plugins")}
            />
          </label>
          <div className="segmented" role="tablist">
            {filters.map((name) => (
              <button
                key={name}
                role="tab"
                aria-selected={filter === name}
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
            <div className="pl-add">
              <button type="button" className="pl-add-button" aria-haspopup="menu" aria-expanded={menuOpen} onClick={() => setMenuOpen((open) => !open)}>
                {tx("Add")}
                <ChevronDownIcon />
              </button>
              {menuOpen && (
                <>
                  <div className="plugin-menu-backdrop" onClick={() => setMenuOpen(false)} />
                  <div className="plugin-menu" role="menu">
                    {addMenuItems.map((item) => (
                      <button
                        key={item.label}
                        role="menuitem"
                        onClick={() => {
                          setMenuOpen(false);
                          item.onClick();
                        }}
                      >
                        {item.label}
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>
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
            <div className="pl-section">
              <h2>{tx("Your plugins")}</h2>
            </div>
            {visiblePlugins.length === 0 && (
              <p className="muted small">{tx("No plugins added yet. Add one from Most used below, or with the Add menu.")}</p>
            )}
            <div className="pl-list">{pluginRows}</div>
            {visiblePlugins.length > 0 && (
              <p className="pl-footnote">{namesLine(visiblePlugins.map((p) => p.name))}</p>
            )}

            <div className="pl-section">
              <h2>{tx("Most used")}</h2>
            </div>
            <div className="mu-grid">
              {(showAllMarket ? catalog : catalog.slice(0, 4)).map((entry) => {
                const added = pluginItems.find((p) => p.catalog_key === entry.key);
                return (
                  <div key={entry.key} className="mu-item">
                    <span className="pl-icon">
                      <PlugIcon />
                    </span>
                    <div className="mu-text">
                      <div className="mu-name">
                        {entry.name}
                        <CheckCircleIcon width={14} height={14} />
                      </div>
                      <div className="mu-desc">{entry.description}</div>
                      <div className="mu-meta">
                        {tx("Plugin")} · {entry.publisher} · {entry.installs} {tx("installs")}
                      </div>
                    </div>
                    {added?.installed ? (
                      <span className="status status-on">{tx("Installed")}</span>
                    ) : (
                      isAdmin && (
                        <button className="primary mu-install" onClick={() => void installFromCatalog(entry)}>
                          {tx("Install")}
                        </button>
                      )
                    )}
                  </div>
                );
              })}
            </div>
            {catalog.length > 4 && (
              <button className="link-button" onClick={() => setShowAllMarket((open) => !open)}>
                {showAllMarket ? tx("Show less") : tx("Show all")}
              </button>
            )}

            {(visibleConnectors.length > 0 || visibleSkills.length > 0) && (
              <>
                <div className="pl-section">
                  <h2>{tx("Connectors and skills")}</h2>
                </div>
                <div className="pl-list">
                  {connectorRows}
                  {skillRows}
                </div>
              </>
            )}
          </>
        ) : (
          <div className="pl-list">
            {pluginRows}
            {connectorRows}
            {skillRows}
          </div>
        )}

        {showSkills && environmentId && <CodexSkills environmentId={environmentId} onError={onError} />}

        {showConnectors && connectorItems.length > 0 && environmentId && (
          <div className="live-check">
            <button
              type="button"
              className="link-button"
              disabled={checking}
              onClick={async () => {
                setChecking(true);
                try {
                  const servers = await environments.mcpServers(environmentId);
                  setLive(Object.fromEntries(servers.map((server) => [server.name, server])));
                } catch (err) {
                  onError(err instanceof Error ? err.message : tx("The servers could not be checked."));
                } finally {
                  setChecking(false);
                }
              }}
            >
              {checking ? tx("Checking…") : tx("Check now")}
            </button>
            <span className="muted small">{tx("Asks codex in this environment what is running now.")}</span>
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
