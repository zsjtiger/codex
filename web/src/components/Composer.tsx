import { useState } from "react";
import { MicIcon, PlusIcon } from "./icons";

const models = ["Haiku 5.5", "Sonnet 5.5", "Opus 5.5"];
const efforts = ["Low", "Medium", "High"];

interface ComposerProps {
  onSend: (text: string) => void;
  disabled: boolean;
  placeholder?: string;
}

/**
 * Message input. Model and effort are shown but not sent yet: the gateway's turn
 * endpoint does not accept them.
 */
export function Composer({ onSend, disabled, placeholder = "How can I help you today?" }: ComposerProps) {
  const [draft, setDraft] = useState("");
  const [model, setModel] = useState(models[0]);
  const [effort, setEffort] = useState(efforts[1]);

  const submit = () => {
    if (!draft.trim() || disabled) return;
    onSend(draft);
    setDraft("");
  };

  return (
    <div className="composer-wrap">
      <form
        className="composer"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
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
          <button type="button" className="icon-button" aria-label="Attach" disabled>
            <PlusIcon />
          </button>
          <div className="composer-right">
            <button type="button" className="icon-button" aria-label="Voice input" disabled>
              <MicIcon />
            </button>
            <button type="submit" className="send" disabled={disabled || !draft.trim()}>
              Send
            </button>
          </div>
        </div>
      </form>
      <div className="composer-options">
        <label className="pill select">
          <select value={model} onChange={(event) => setModel(event.target.value)} aria-label="Model">
            {models.map((m) => (
              <option key={m}>{m}</option>
            ))}
          </select>
        </label>
        <label className="pill select">
          <select value={effort} onChange={(event) => setEffort(event.target.value)} aria-label="Effort">
            {efforts.map((e) => (
              <option key={e}>{e}</option>
            ))}
          </select>
        </label>
      </div>
    </div>
  );
}
