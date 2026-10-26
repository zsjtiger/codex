import { useRef, useState } from "react";
import type { Project } from "../api/types";
import type { SendOptions } from "../state/useCodex";
import { ChevronIcon, PlusIcon } from "./icons";

const MAX_ATTACHMENT_BYTES = 200_000;

interface ComposerProps {
  onSend: (text: string, options: SendOptions) => void;
  onStop?: () => void;
  running?: boolean;
  disabled: boolean;
  placeholder?: string;
  projects: Project[];
  projectId: string | null;
  onProjectChange?: (projectId: string | null) => void;
  upload: (file: File) => Promise<string>;
  onError: (message: string) => void;
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
  const { onSend, onStop, running, disabled, placeholder = "How can I help you today?", projects, projectId, onProjectChange, upload, onError } = props;
  const [draft, setDraft] = useState("");
  const [model, setModel] = useState("");
  const [effort, setEffort] = useState("");
  const [pending, setPending] = useState<Pending[]>([]);
  const [uploading, setUploading] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [attachOpen, setAttachOpen] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const submit = () => {
    if ((!draft.trim() && pending.length === 0) || disabled || uploading) return;
    onSend(draft, {
      model: model.trim() || undefined,
      effort: effort || undefined,
      attachment_ids: pending.length ? pending.map((p) => p.id) : undefined,
    });
    setDraft("");
    setPending([]);
  };

  const attach = async (file: File) => {
    if (file.size > MAX_ATTACHMENT_BYTES) {
      onError(`${file.name} is larger than ${MAX_ATTACHMENT_BYTES / 1000} KB.`);
      return;
    }
    setUploading(true);
    try {
      const id = await upload(file);
      setPending((prev) => [...prev, { id, name: file.name }]);
    } catch (err) {
      onError(err instanceof Error ? err.message : "Upload failed.");
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
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              submit();
            }
          }}
        />
        <div className="composer-bar">
          <div className="add-menu">
            <button type="button" className="icon-button" aria-label="Attach" onClick={() => setAttachOpen(!attachOpen)} disabled={uploading}>
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
                    Attach a text file
                  </button>
                </li>
              </ul>
            )}
            <input
              ref={fileRef}
              type="file"
              hidden
              accept="text/*,.json,.md,.csv,.py,.ts,.tsx,.js,.rs,.go,.java,.c,.cpp,.h,.yaml,.yml,.toml"
              onChange={(event) => {
                const file = event.target.files?.[0];
                event.target.value = "";
                if (file) void attach(file);
              }}
            />
          </div>
          <div className="composer-right">
            {uploading && <span className="muted small">Uploading…</span>}
            {running && onStop ? (
              <button type="button" className="stop" onClick={onStop}>
                Stop
              </button>
            ) : (
              <button type="submit" className="send" disabled={disabled || uploading || (!draft.trim() && pending.length === 0)}>
                Send
              </button>
            )}
          </div>
        </div>
      </form>

      <div className="composer-options">
        {onProjectChange ? (
          <div className="add-menu">
            <button type="button" className="pill" onClick={() => setPickerOpen(!pickerOpen)} aria-expanded={pickerOpen}>
              {projectName ?? "No project"} <ChevronIcon />
            </button>
            {pickerOpen && (
              <ul className="add-menu-list composer-menu" role="menu">
                <li role="menuitem">
                  <button type="button" className="menu-action" onClick={() => { onProjectChange(null); setPickerOpen(false); }}>
                    No project
                  </button>
                </li>
                {projects.map((project) => (
                  <li key={project.id} role="menuitem">
                    <button type="button" className="menu-action" onClick={() => { onProjectChange(project.id); setPickerOpen(false); }}>
                      {project.name}
                    </button>
                  </li>
                ))}
                {projects.length === 0 && <li className="menu-note">Create a project in Projects first.</li>}
              </ul>
            )}
          </div>
        ) : (
          <span />
        )}
        <div className="composer-options-right">
          <input
            className="pill-input"
            value={model}
            onChange={(event) => setModel(event.target.value)}
            placeholder="Model (default)"
            aria-label="Model"
            maxLength={100}
          />
          <select className="pill-select" value={effort} onChange={(event) => setEffort(event.target.value)} aria-label="Reasoning effort">
            <option value="">Effort (default)</option>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
          </select>
        </div>
      </div>
    </div>
  );
}
