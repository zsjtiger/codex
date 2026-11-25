import { useState } from "react";
import type { Connector, Plugin } from "../api/types";
import { tx } from "../i18n";

const tabs = ["Overview", "Files", "Permissions"] as const;
type Tab = (typeof tabs)[number];

interface PluginDetailProps {
  plugin: Plugin;
  /** The workspace's connectors, for the status of the ones this plugin adds. */
  connectors: Connector[];
  isAdmin: boolean;
  onBack: () => void;
  onInstall: () => void;
  onUninstall: () => void;
  onDelete: () => void;
}

/** The host a connector address points at, shown instead of the full address. */
function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** A skill as the file the model reads: its name and description in front, then its instructions. */
function skillFile(name: string, description: string, instructions: string): string {
  return `---\nname: ${name}\ndescription: ${description}\n---\n\n${instructions}\n`;
}

function connectorState(connector: Connector | undefined): string {
  if (!connector) return tx("Not added yet");
  if (!connector.enabled) return tx("Disabled");
  return connector.status === "ready" ? tx("Connected") : tx("Not checked");
}

/** One plugin: what it adds, the files that describe it, and what its connectors may do. */
export function PluginDetail({ plugin, connectors, isAdmin, onBack, onInstall, onUninstall, onDelete }: PluginDetailProps) {
  const [tab, setTab] = useState<Tab>("Overview");
  const [openSkill, setOpenSkill] = useState<number | null>(null);
  const [fileKey, setFileKey] = useState("manifest");
  const { skills, connectors: pluginConnectors } = plugin.manifest;
  const installed = plugin.installed;
  const byName = (name: string) => connectors.find((c) => c.name === name);
  const hosts = [...new Set(pluginConnectors.map((c) => hostOf(c.url)))];

  // The plugin's files, made from its contents: the manifest, then one SKILL.md per skill.
  const files = [
    {
      key: "manifest",
      path: ".claude-plugin/plugin.json",
      content: JSON.stringify(
        {
          name: plugin.name,
          description: plugin.description,
          skills: skills.map((s) => s.name),
          connectors: pluginConnectors.map((c) => ({ name: c.name, url: c.url })),
        },
        null,
        2,
      ),
    },
    ...skills.map((s, i) => ({
      key: `skill-${i}`,
      path: `skills/${s.name}/SKILL.md`,
      content: skillFile(s.name, s.description, s.instructions),
    })),
  ];
  const shown = files.find((f) => f.key === fileKey) ?? files[0];

  return (
    <>
      <button className="link-button" onClick={onBack}>
        {tx("← Plugins")}
      </button>

      <div className="plugin-detail-head">
        <span className="plugin-mark plugin-mark-large">{(plugin.name.trim().charAt(0) || "?").toUpperCase()}</span>
        <div className="plugin-detail-title">
          <h2>{plugin.name}</h2>
          <span className="muted small">{plugin.catalog_key ? tx("By Baize") : tx("By you")}</span>
        </div>
        <div className="plugin-detail-actions">
          <span className={`status ${installed ? "status-on" : ""}`}>{tx(installed ? "Installed" : "Not installed")}</span>
          {isAdmin &&
            (installed ? (
              <button onClick={onUninstall}>{tx("Uninstall")}</button>
            ) : (
              <button className="primary" onClick={onInstall}>
                {tx("Install")}
              </button>
            ))}
          {isAdmin && <button onClick={onDelete}>{tx("Delete")}</button>}
        </div>
      </div>

      <div className="tabs" role="tablist">
        {tabs.map((name) => (
          <button key={name} role="tab" aria-selected={tab === name} className={`tab ${tab === name ? "tab-active" : ""}`} onClick={() => setTab(name)}>
            {tx(name)}
          </button>
        ))}
      </div>

      {tab === "Overview" && (
        <div className="plugin-detail-body">
          <p>{plugin.description || tx("No description.")}</p>

          <h3 className="plugin-detail-heading">{tx("Skills")}</h3>
          {skills.length === 0 && <p className="muted small">{tx("This plugin adds no skills.")}</p>}
          {skills.map((skill, i) => (
            <div key={i} className="plugin-skill">
              <button className="plugin-skill-head" onClick={() => setOpenSkill(openSkill === i ? null : i)} aria-expanded={openSkill === i}>
                <strong>{skill.name}</strong>
                <span className="muted small">{skill.description}</span>
              </button>
              {openSkill === i && <pre className="plugin-skill-text">{skill.instructions}</pre>}
            </div>
          ))}

          <h3 className="plugin-detail-heading">{tx("Connectors")}</h3>
          {pluginConnectors.length === 0 && <p className="muted small">{tx("This plugin adds no connectors.")}</p>}
          {pluginConnectors.map((c, i) => (
            <div key={i} className="plugin-connector-row">
              <strong>{c.name}</strong>
              <code>{hostOf(c.url)}</code>
              <span className="muted small">{connectorState(byName(c.name))}</span>
            </div>
          ))}

          <div className="plugin-facts">
            <div>
              <span className="muted small">{tx("Developer")}</span>
              <div>{plugin.catalog_key ? "Baize" : tx("You")}</div>
            </div>
            <div>
              <span className="muted small">{tx("Safety and access")}</span>
              <ul>
                {skills.length > 0 && <li>{tx("Adds instructions for the model")}</li>}
                {hosts.map((h) => (
                  <li key={h}>
                    {tx("Connects to")} {h}
                  </li>
                ))}
                {skills.length === 0 && hosts.length === 0 && <li>{tx("Adds nothing to chats")}</li>}
              </ul>
            </div>
          </div>
        </div>
      )}

      {tab === "Files" && (
        <div className="plugin-files">
          <div className="plugin-tree">
            {files.map((f) => (
              <button key={f.key} className={`plugin-file ${shown.key === f.key ? "plugin-file-active" : ""}`} onClick={() => setFileKey(f.key)}>
                {f.path}
              </button>
            ))}
          </div>
          <pre className="plugin-file-view">{shown.content}</pre>
        </div>
      )}

      {tab === "Permissions" && (
        <div className="plugin-detail-body">
          {pluginConnectors.length === 0 ? (
            <p className="muted small">{tx("This plugin adds no connectors, so it uses no tools.")}</p>
          ) : (
            <>
              {pluginConnectors.map((c, i) => (
                <div key={i} className="plugin-connector-row">
                  <strong>{c.name}</strong>
                  <span className="muted small">{connectorState(byName(c.name))}</span>
                </div>
              ))}
              <p className="muted small">
                {tx("A connector's tools are listed once it is connected. Codex asks before using them, according to the workspace's approval setting.")}
              </p>
            </>
          )}
        </div>
      )}
    </>
  );
}
