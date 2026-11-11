import { useEffect, useRef, useState } from "react";
import { type ModelOption, environments as environmentApi } from "../api/resources";
import type { Project, Skill } from "../api/types";
import { useI18n } from "../i18n";

/** The browser's speech recognition, when it has one (Chromium and Safari do; some webviews do not). */
interface SpeechRecognitionLike {
  lang: string;
  interimResults: boolean;
  continuous: boolean;
  onresult: ((event: { results: ArrayLike<ArrayLike<{ transcript: string }>> }) => void) | null;
  onend: (() => void) | null;
  onerror: (() => void) | null;
  start(): void;
  stop(): void;
}

function speechRecognitionConstructor(): (new () => SpeechRecognitionLike) | null {
  const scope = window as unknown as { SpeechRecognition?: new () => SpeechRecognitionLike; webkitSpeechRecognition?: new () => SpeechRecognitionLike };
  return scope.SpeechRecognition ?? scope.webkitSpeechRecognition ?? null;
}

function MicIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
    </svg>
  );
}
import type { SendOptions } from "../state/useCodex";
import { ChevronIcon, PlusIcon, ArrowUpIcon } from "./icons";
import { tx } from "../i18n";

/** Text files are sent to the model, so they are kept small. Word and PDF files are only shown. */
const MAX_TEXT_ATTACHMENT_BYTES = 200_000;
const MAX_BINARY_ATTACHMENT_BYTES = 20_000_000;

function attachmentLimit(file: File): number {
  const name = file.name.toLowerCase();
  const binary = name.endsWith(".pdf") || name.endsWith(".docx");
  return binary ? MAX_BINARY_ATTACHMENT_BYTES : MAX_TEXT_ATTACHMENT_BYTES;
}

interface ComposerProps {
  /** The environment whose codex runtime offers the models. */
  environmentId?: string | null;
  onSend: (text: string, options: SendOptions) => void;
  onStop?: () => void;
  running?: boolean;
  disabled: boolean;
  placeholder?: string;
  projects: Project[];
  projectId: string | null;
  onProjectChange?: (projectId: string | null) => void;
  /** Creates a project from the picker and returns its id. */
  onCreateProject?: (name: string) => Promise<string>;
  /** Enabled skills in the workspace. Typing "/" offers them. */
  skills: Skill[];
  upload: (file: File) => Promise<string>;
  onError: (message: string) => void;
}

const escapeRegExp = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** Skills whose "/name" appears as a word in the message. */
function skillsInMessage(text: string, skills: Skill[]): string[] {
  return skills
    .filter((skill) => new RegExp(`(^|\\s)/${escapeRegExp(skill.name)}(?=\\s|$)`).test(text))
    .map((skill) => skill.id);
}

interface Pending {
  id: string;
  name: string;
}

/**
 * Message input. Sends the optional model and effort with each turn, uploads text
 * files as attachments, and offers Stop while a turn is running.
 */
export function Composer(props: ComposerProps) {
  const { t } = useI18n();
  const { environmentId, onSend, onStop, running, disabled, placeholder = t("home.placeholder"), projects, projectId, onProjectChange, onCreateProject, skills, upload, onError } = props;
  const [draft, setDraft] = useState("");
  const [listening, setListening] = useState(false);
  const recognizerRef = useRef<SpeechRecognitionLike | null>(null);
  const SpeechInput = speechRecognitionConstructor();
  const { locale } = useI18n();

  // Starts or stops dictation. The spoken text is added to the message, which can still be edited.
  const toggleVoice = () => {
    if (!SpeechInput) return;
    if (listening) {
      recognizerRef.current?.stop();
      return;
    }
    const recognizer = new SpeechInput();
    recognizer.lang = locale === "zh" ? "zh-CN" : "en-US";
    recognizer.interimResults = false;
    recognizer.continuous = false;
    recognizer.onresult = (event) => {
      const spoken = Array.from(event.results)
        .map((result) => result[0]?.transcript ?? "")
        .join(" ")
        .trim();
      if (spoken) setDraft((current) => (current ? `${current} ${spoken}` : spoken));
    };
    recognizer.onend = () => {
      setListening(false);
      recognizerRef.current = null;
    };
    recognizer.onerror = () => {
      setListening(false);
      recognizerRef.current = null;
      onError(tx("Voice input could not hear anything. Check the microphone and try again."));
    };
    recognizerRef.current = recognizer;
    setListening(true);
    recognizer.start();
  };
  const [menuIndex, setMenuIndex] = useState(0);
  const [menuDismissed, setMenuDismissed] = useState(false);
  const [model, setModel] = useState("");
  const [effort, setEffort] = useState("");
  const [models, setModels] = useState<ModelOption[]>([]);

  useEffect(() => {
    let current = true;
    if (!environmentId) return;
    environmentApi
      .models(environmentId)
      .then((list) => current && setModels(list))
      .catch(() => current && setModels([]));
    return () => {
      current = false;
    };
  }, [environmentId]);

  // Effort options follow the chosen model. A default model uses the first listed model's.
  const chosen = models.find((m) => m.model === model) ?? models.find((m) => m.is_default);
  // Models that list no efforts (for example custom providers) get the standard three.
  const efforts = chosen?.efforts.length ? chosen.efforts : ["low", "medium", "high"];
  useEffect(() => {
    if (effort && !efforts.includes(effort)) setEffort("");
  }, [efforts, effort]);
  const [pending, setPending] = useState<Pending[]>([]);
  const [uploading, setUploading] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [projectQuery, setProjectQuery] = useState("");
  const [newProjectName, setNewProjectName] = useState<string | null>(null);
  const [attachOpen, setAttachOpen] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const submit = () => {
    if ((!draft.trim() && pending.length === 0) || disabled || uploading) return;
    const skillIds = skillsInMessage(draft, skills);
    onSend(draft, {
      model: model.trim() || undefined,
      effort: effort || undefined,
      attachment_ids: pending.length ? pending.map((p) => p.id) : undefined,
      skill_ids: skillIds.length ? skillIds : undefined,
    });
    setDraft("");
    setPending([]);
  };

  // "/" at the start of an otherwise empty message opens the menu, filtered by what follows it.
  const slash = /^\/(\S*)$/.exec(draft);
  const menuOpen = slash !== null && !menuDismissed;
  const query = (slash?.[1] ?? "").toLowerCase();
  const menuItems: { key: string; label: string; run: () => void }[] = [
    {
      key: "attach",
      label: tx("Add files"),
      run: () => {
        setDraft("");
        fileRef.current?.click();
      },
    },
    ...skills
      .filter((skill) => skill.name.toLowerCase().includes(query))
      .map((skill) => ({
        key: skill.id,
        label: skill.name,
        run: () => setDraft(`/${skill.name} `),
      })),
  ];
  const highlighted = Math.min(menuIndex, Math.max(menuItems.length - 1, 0));

  const attach = async (file: File) => {
    const limit = attachmentLimit(file);
    if (file.size > limit) {
      onError(`${file.name} ${tx("is larger than")} ${limit / 1000} KB.`);
      return;
    }
    setUploading(true);
    try {
      const id = await upload(file);
      setPending((prev) => [...prev, { id, name: file.name }]);
    } catch (err) {
      onError(err instanceof Error ? err.message : tx("Upload failed."));
    } finally {
      setUploading(false);
    }
  };

  const projectName = projects.find((p) => p.id === projectId)?.name;

  return (
    <div className="composer-wrap">
      <form
        className="composer"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        {pending.length > 0 && (
          <div className="chips">
            {pending.map((p) => (
              <span key={p.id} className="chip">
                {p.name}
                <button type="button" aria-label={`Remove ${p.name}`} onClick={() => setPending(pending.filter((x) => x.id !== p.id))}>
                  ×
                </button>
              </span>
            ))}
          </div>
        )}
        <textarea
          className="composer-input"
          placeholder={placeholder}
          rows={2}
          value={draft}
          disabled={disabled}
          onChange={(event) => {
            setDraft(event.target.value);
            setMenuDismissed(false);
            setMenuIndex(0);
          }}
          onKeyDown={(event) => {
            if (menuOpen && menuItems.length > 0) {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                const step = event.key === "ArrowDown" ? 1 : -1;
                setMenuIndex((highlighted + step + menuItems.length) % menuItems.length);
                return;
              }
              if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                menuItems[highlighted].run();
                return;
              }
            }
            if (menuOpen && event.key === "Escape") {
              setMenuDismissed(true);
              return;
            }
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              submit();
            }
          }}
        />
        {menuOpen && (
          <ul className="slash-menu" role="listbox" aria-label={tx("Skills and actions")}>
            {menuItems.map((item, index) => (
              <li
                key={item.key}
                role="option"
                aria-selected={index === highlighted}
                className={index === highlighted ? "slash-item slash-item-active" : "slash-item"}
                onMouseDown={(event) => {
                  event.preventDefault();
                  item.run();
                }}
                onMouseEnter={() => setMenuIndex(index)}
              >
                {item.label}
              </li>
            ))}
            {menuItems.length === 1 && <li className="slash-empty muted">{tx("No matching skills")}</li>}
          </ul>
        )}
        <div className="composer-bar">
          <div className="add-menu">
            <button type="button" className="icon-button" aria-label={tx("Attach")} onClick={() => setAttachOpen(!attachOpen)} disabled={uploading}>
              <PlusIcon />
            </button>
            {attachOpen && (
              <ul className="add-menu-list composer-menu" role="menu">
                <li role="menuitem">
                  <button
                    type="button"
                    className="menu-action"
                    onClick={() => {
                      setAttachOpen(false);
                      fileRef.current?.click();
                    }}
                  >
                    {tx("Attach a file")}
                  </button>
                </li>
              </ul>
            )}
            <input
              ref={fileRef}
              type="file"
              hidden
              accept="text/*,.json,.md,.csv,.py,.ts,.tsx,.js,.rs,.go,.java,.c,.cpp,.h,.yaml,.yml,.toml,.pdf,.docx"
              onChange={(event) => {
                const file = event.target.files?.[0];
                event.target.value = "";
                if (file) void attach(file);
              }}
            />
          </div>
          <div className="composer-right">
            {uploading && <span className="muted small">{tx("Uploading…")}</span>}
          <div className="composer-options-right">
            <select className="pill-select" value={model} onChange={(event) => setModel(event.target.value)} aria-label={tx("Model")}>
              <option value="">{t("composer.model")}</option>
              {models.map((option) => (
                <option key={option.model} value={option.model}>
                  {option.display_name || option.model}
                </option>
              ))}
            </select>
            <select className="pill-select" value={effort} onChange={(event) => setEffort(event.target.value)} aria-label={tx("Reasoning effort")}>
              <option value="">{t("composer.effort")}</option>
              {efforts.map((level) => (
                <option key={level} value={level}>
                  {tx(level.charAt(0).toUpperCase() + level.slice(1))}
                </option>
              ))}
            </select>
          </div>
            {SpeechInput && (
              <button
                type="button"
                className={`icon-button mic-button ${listening ? "mic-button-on" : ""}`}
                aria-label={listening ? tx("Stop voice input") : tx("Voice input")}
                aria-pressed={listening}
                title={listening ? tx("Stop voice input") : tx("Voice input")}
                onClick={toggleVoice}
              >
                <MicIcon />
              </button>
            )}
            {running && onStop ? (
              <button type="button" className="stop" onClick={onStop}>
                {tx("Stop")}
              </button>
            ) : (
              <button type="submit" className="send" aria-label={tx("Send")} title={tx("Send")} disabled={disabled || uploading || (!draft.trim() && pending.length === 0)}>
                <ArrowUpIcon />
              </button>
            )}
          </div>
        </div>
      </form>

      <div className="composer-options">
        {onProjectChange ? (
          <div className="add-menu">
            <button type="button" className="pill" onClick={() => setPickerOpen(!pickerOpen)} aria-expanded={pickerOpen}>
              {projectName ?? t("composer.project")} <ChevronIcon />
            </button>
            {pickerOpen && (
              <div className="add-menu-list composer-menu project-picker" role="menu">
                <input
                  className="project-search"
                  placeholder={tx("Search projects")}
                  aria-label={tx("Search projects")}
                  value={projectQuery}
                  onChange={(event) => setProjectQuery(event.target.value)}
                  autoFocus
                />
                <div className="project-picker-list">
                  {projects
                    .filter((project) => project.name.toLowerCase().includes(projectQuery.trim().toLowerCase()))
                    .map((project) => (
                      <button
                        key={project.id}
                        type="button"
                        role="menuitem"
                        className={`menu-action ${project.id === projectId ? "menu-action-current" : ""}`}
                        onClick={() => {
                          onProjectChange(project.id);
                          setPickerOpen(false);
                        }}
                      >
                        {project.name}
                      </button>
                    ))}
                  {projectQuery.trim() === "" && (
                    <button type="button" role="menuitem" className="menu-action" onClick={() => { onProjectChange(null); setPickerOpen(false); }}>
                      {t("composer.noProject")}
                    </button>
                  )}
                </div>
                <div className="project-picker-footer">
                  {onCreateProject && newProjectName === null && (
                    <button type="button" role="menuitem" className="menu-action" onClick={() => setNewProjectName("")}>
                      {tx("+ New project")}
                    </button>
                  )}
                  {onCreateProject && newProjectName !== null && (
                    <form
                      className="project-new-form"
                      onSubmit={async (event) => {
                        event.preventDefault();
                        const name = newProjectName.trim();
                        if (!name) return;
                        try {
                          const id = await onCreateProject(name);
                          onProjectChange(id);
                          setNewProjectName(null);
                          setPickerOpen(false);
                        } catch (err) {
                          onError?.(err instanceof Error ? err.message : tx("Could not create the project."));
                        }
                      }}
                    >
                      <input
                        autoFocus
                        value={newProjectName}
                        onChange={(event) => setNewProjectName(event.target.value)}
                        placeholder={tx("Project name")}
                        maxLength={100}
                        aria-label={tx("New project name")}
                      />
                      <button type="submit" className="primary" disabled={!newProjectName.trim()}>{tx("Create")}</button>
                    </form>
                  )}
                </div>
              </div>
            )}
          </div>
        ) : (
          <span />
        )}

      </div>
    </div>
  );
}
