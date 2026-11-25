import { useState } from "react";
import type { Connector, Plugin } from "../api/types";
import { MarkdownView } from "./Markdown";
import { CheckCircleIcon, ChevronDownIcon, CodeIcon, CopyIcon, DownloadIcon, EyeIcon, FileIcon, FolderIcon, InfoIcon, PlugIcon } from "./icons";
import { RowMenu, type MenuItem } from "./RowMenu";
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

/** The host a connector address points at. */
function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** A skill as the file the model reads: name and description in front, then the instructions. */
function skillFile(name: string, description: string, instructions: string): string {
  return `---\nname: ${name}\ndescription: ${description}\n---\n\n${instructions}\n`;
}

/**
 * A skill file as it reads: its front matter (name and description) in a small block, then the
 * instructions as Markdown. Other Markdown is shown as Markdown whole.
 */
function SkillPreview({ content }: { content: string }) {
  const match = /^---\n([\s\S]*?)\n---\n\n?([\s\S]*)$/.exec(content);
  if (!match) return <MarkdownView source={content} />;
  return (
    <>
      <pre className="pd-frontmatter">{match[1]}</pre>
      <MarkdownView source={match[2]} />
    </>
  );
}

/** The status of a connector the plugin adds, as the workspace sees it. */
function connectorState(connector: Connector | undefined): string {
  if (!connector) return tx("Not added yet");
  if (!connector.enabled) return tx("Disabled");
  return connector.status === "ready" ? tx("Connected") : tx("Not checked");
}

function signInState(connector: Connector | undefined): string {
  if (!connector) return tx("Not added yet");
  switch (connector.oauth_state) {
    case "signed_in":
      return tx("Signed in");
    case "pending":
      return tx("Waiting for sign-in…");
    case "failed":
      return tx("Sign-in failed");
    default:
      return tx("Not started");
  }
}

/** One plugin: its overview, the files that describe it, and what its connectors may do. */
export function PluginDetail({ plugin, connectors, isAdmin, onBack, onInstall, onUninstall, onDelete }: PluginDetailProps) {
  const [tab, setTab] = useState<Tab>("Overview");
  const [openSkills, setOpenSkills] = useState<Record<number, boolean>>({});
  const [fileKey, setFileKey] = useState("manifest");
  const [view, setView] = useState<"preview" | "code">("preview");
  const { skills, connectors: pluginConnectors } = plugin.manifest;
  const installed = plugin.installed;
  const byName = (name: string) => connectors.find((c) => c.name === name);
  const hosts = [...new Set(pluginConnectors.map((c) => hostOf(c.url)))];
  const developer = plugin.catalog_key ? "Baize" : tx("You");

  // The plugin's files, made from its contents: the manifest, then one SKILL.md per skill.
  const manifestJson = JSON.stringify(
    {
      name: plugin.name,
      description: plugin.description,
      skills: skills.map((s) => s.name),
      connectors: pluginConnectors.map((c) => ({ name: c.name, url: c.url })),
    },
    null,
    2,
  );
  const files: Record<string, { path: string; content: string }> = {
    manifest: { path: ".claude-plugin/plugin.json", content: manifestJson },
    ...Object.fromEntries(
      skills.map((s, i) => [`skill-${i}`, { path: `skills/${s.name}/SKILL.md`, content: skillFile(s.name, s.description, s.instructions) }]),
    ),
  };
  const shown = files[fileKey] ?? files.manifest;
  const isMarkdown = shown.path.endsWith(".md");

  const download = () => {
    const blob = new Blob([manifestJson], { type: "application/json" });
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob);
    link.download = `${plugin.name}.json`;
    link.click();
    URL.revokeObjectURL(link.href);
  };

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // Clipboard access can be refused; the text stays selectable.
    }
  };

  const menu: MenuItem[] = [];
  if (isAdmin) {
    menu.push(
      installed ? { label: tx("Uninstall"), onClick: onUninstall } : { label: tx("Install"), onClick: onInstall },
      { label: tx("Delete"), danger: true, onClick: onDelete },
    );
  }

  return (
    <>
      <button className="link-button" onClick={onBack}>
        {tx("← Plugins")}
      </button>

      <div className="pd-head">
        <span className="pd-icon">
          <PlugIcon width={24} height={24} />
        </span>
        <div className="pd-title">
          <h1>{plugin.name}</h1>
          <div className="pd-sub">{plugin.catalog_key ? tx("By Baize") : tx("By you")}</div>
        </div>
        <div className="pd-actions">
          <span className={`pd-added ${installed ? "pd-added-on" : ""}`}>
            {installed && <CheckCircleIcon width={15} height={15} />}
            {tx(installed ? "Added" : "Not added")}
          </span>
          <RowMenu items={menu} label={plugin.name} />
        </div>
      </div>

      <div className="pd-tabs" role="tablist">
        {tabs.map((name) => (
          <button key={name} role="tab" aria-selected={tab === name} onClick={() => setTab(name)}>
            {tx(name)}
          </button>
        ))}
      </div>

      {tab === "Overview" && (
        <>
          <p className="pd-desc">{plugin.description || tx("No description.")}</p>

          <h3 className="pd-h">{tx("Skills")}</h3>
          {skills.length === 0 && <p className="muted small">{tx("This plugin adds no skills.")}</p>}
          {skills.length > 0 && (
            <div className="pd-card">
              {skills.map((skill, i) => (
                <div key={i}>
                  <div className="pd-card-row">
                    <span className="pd-card-icon">
                      <FileIcon width={18} height={18} />
                    </span>
                    <div className="pd-card-body">
                      <div className="pd-card-name">{skill.name}</div>
                      <div className="pd-card-desc">{skill.description}</div>
                    </div>
                    <button
                      type="button"
                      className="row-menu-button"
                      aria-expanded={!!openSkills[i]}
                      aria-label={tx("Show instructions")}
                      title={tx("Show instructions")}
                      onClick={() => setOpenSkills((current) => ({ ...current, [i]: !current[i] }))}
                    >
                      <ChevronDownIcon className={openSkills[i] ? "pd-rotated" : undefined} />
                    </button>
                  </div>
                  {openSkills[i] && <pre className="pd-skill-text">{skill.instructions}</pre>}
                  <div className="pd-file-line">
                    <span>{tx("File")}</span>
                    <code>skills/{skill.name}/SKILL.md</code>
                  </div>
                </div>
              ))}
            </div>
          )}

          <h3 className="pd-h">{tx("Connectors")}</h3>
          {pluginConnectors.length === 0 && <p className="muted small">{tx("This plugin adds no connectors.")}</p>}
          {pluginConnectors.map((c, i) => {
            const live = byName(c.name);
            const state = connectorState(live);
            return (
              <div key={i} className="pd-card pd-connector">
                <div className="pd-card-row">
                  <span className="pd-card-icon">
                    <PlugIcon width={18} height={18} />
                  </span>
                  <div className="pd-card-body">
                    <div className="pd-card-name">{c.name}</div>
                  </div>
                  <span className={`pd-state ${state === tx("Connected") ? "pd-state-on" : ""}`}>
                    {state === tx("Connected") && <CheckCircleIcon width={14} height={14} />}
                    {state}
                  </span>
                </div>
                <div className="pd-field">
                  <span>{tx("Connector URL")}</span>
                  <span>
                    <code>{c.url}</code>
                    <button type="button" className="row-menu-button pd-copy" aria-label={tx("Copy")} title={tx("Copy")} onClick={() => void copy(c.url)}>
                      <CopyIcon width={15} height={15} />
                    </button>
                  </span>
                </div>
                <div className="pd-field">
                  <span>{tx("Sign-in")}</span>
                  <span>{signInState(live)}</span>
                </div>
              </div>
            );
          })}

          <div className="pd-facts">
            <div>
              <h4>
                <InfoIcon width={14} height={14} />
                {tx("Developer")}
              </h4>
              <p>{developer}</p>
            </div>
            <div>
              <h4>
                <InfoIcon width={14} height={14} />
                {tx("Safety and access")}
              </h4>
              <ul className="pd-safety">
                {skills.length > 0 && pluginConnectors.length === 0 && <li>{tx("Only adds instructions for the model")}</li>}
                {skills.length > 0 && pluginConnectors.length > 0 && <li>{tx("Adds instructions for the model")}</li>}
                {hosts.map((h) => (
                  <li key={h}>
                    {tx("Connects to")} {h}
                  </li>
                ))}
                {skills.length === 0 && hosts.length === 0 && <li>{tx("Adds nothing to chats")}</li>}
              </ul>
            </div>
          </div>
        </>
      )}

      {tab === "Files" && (
        <div className="pd-files-page">
          <div className="pd-files-bar">
            <span className="pd-version">{plugin.name} · {tx("current")}</span>
            <button type="button" className="row-menu-button" aria-label={tx("Download")} title={tx("Download")} onClick={download}>
              <DownloadIcon width={17} height={17} />
            </button>
          </div>
          <div className="pd-files">
            <div className="pd-tree">
              <div className="pd-tree-row pd-tree-folder">
                <FolderIcon width={16} height={16} />
                .claude-plugin
              </div>
              <button type="button" className={`pd-tree-row pd-tree-file pd-tree-indent-1 ${fileKey === "manifest" ? "active" : ""}`} onClick={() => setFileKey("manifest")}>
                plugin.json
              </button>
              {skills.length > 0 && (
                <div className="pd-tree-row pd-tree-folder">
                  <FolderIcon width={16} height={16} />
                  skills
                </div>
              )}
              {skills.map((s, i) => (
                <div key={i}>
                  <div className="pd-tree-row pd-tree-folder pd-tree-indent-1">
                    <FolderIcon width={16} height={16} />
                    {s.name}
                  </div>
                  <button
                    type="button"
                    className={`pd-tree-row pd-tree-file pd-tree-indent-2 ${fileKey === `skill-${i}` ? "active" : ""}`}
                    onClick={() => setFileKey(`skill-${i}`)}
                  >
                    SKILL.md
                  </button>
                </div>
              ))}
            </div>
            <div className="pd-viewer">
              <div className="pd-viewer-bar">
                <code>/{shown.path}</code>
                {isMarkdown && (
                  <span className="pd-view-toggle">
                    <button type="button" className={view === "preview" ? "active" : ""} aria-label={tx("Preview")} title={tx("Preview")} onClick={() => setView("preview")}>
                      <EyeIcon width={15} height={15} />
                    </button>
                    <button type="button" className={view === "code" ? "active" : ""} aria-label={tx("Code")} title={tx("Code")} onClick={() => setView("code")}>
                      <CodeIcon width={15} height={15} />
                    </button>
                  </span>
                )}
              </div>
              <div className="pd-viewer-body">
                {isMarkdown && view === "preview" ? <SkillPreview content={shown.content} /> : <pre>{shown.content}</pre>}
              </div>
            </div>
          </div>
        </div>
      )}

      {tab === "Permissions" && (
        <>
          {pluginConnectors.length === 0 && (
            <p className="muted small">{tx("This plugin adds no connectors, so it uses no tools.")}</p>
          )}
          {pluginConnectors.map((c, i) => (
            <div key={i}>
              <h3 className="pd-h">{c.name}</h3>
              <div className="pd-card">
                <div className="pd-field">
                  <span>{tx("Connection")}</span>
                  <span>{connectorState(byName(c.name))}</span>
                </div>
                <div className="pd-field">
                  <span>{tx("Tools")}</span>
                  <span className="muted">{tx("Listed once the connector connects")}</span>
                </div>
                <div className="pd-field">
                  <span>{tx("Approval")}</span>
                  <span>{tx("Codex asks before using them, according to the workspace's approval setting.")}</span>
                </div>
              </div>
            </div>
          ))}
        </>
      )}
    </>
  );
}
