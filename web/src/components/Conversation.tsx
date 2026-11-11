import { useEffect, useRef, useState } from "react";
import { MarkdownView } from "./Markdown";
import { CopyIcon, RetryIcon, SpeakIcon, ThumbDownIcon, ThumbUpIcon } from "./icons";
import type { Activity, Approval } from "../state/useCodex";
import { ChevronIcon } from "./icons";
import { usePreview } from "./Preview";
import { attachments as attachmentApi } from "../api/resources";
import type { Turn } from "../api/types";

/** "just now", "5 min ago", "3 h ago", or the date for older replies. */
function relativeTime(iso: string): string {
  const seconds = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  return new Date(iso).toLocaleDateString();
}

interface ReplyActionsProps {
  turn: Turn;
  reply: string;
  onFeedback: (turnId: string, rating: "up" | "down" | null) => void;
  onRegenerate: (input: string) => void;
}

/**
 * Copies text to the clipboard. The async clipboard API exists only in secure contexts (https,
 * localhost and the desktop app), so plain-http pages fall back to a hidden text area.
 */
async function copyText(text: string): Promise<boolean> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through to the copy command below.
    }
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.opacity = "0";
  document.body.appendChild(area);
  area.select();
  const ok = document.execCommand("copy");
  document.body.removeChild(area);
  return ok;
}

/** Copy, read aloud, rate, and regenerate for an assistant reply. */
function ReplyActions({ turn, reply, onFeedback, onRegenerate }: ReplyActionsProps) {
  const [copied, setCopied] = useState(false);
  const [speaking, setSpeaking] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.speechSynthesis?.cancel(), []);

  const copy = async () => {
    const done = () => {
      setCopied(true);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setCopied(false), 1500);
    };
    if (await copyText(reply)) done();
  };

  const speak = () => {
    const synth = window.speechSynthesis;
    if (!synth) return;
    if (speaking) {
      synth.cancel();
      setSpeaking(false);
      return;
    }
    const utterance = new SpeechSynthesisUtterance(reply);
    utterance.onend = () => setSpeaking(false);
    utterance.onerror = () => setSpeaking(false);
    setSpeaking(true);
    synth.speak(utterance);
  };

  const rate = (rating: "up" | "down") => onFeedback(turn.id, turn.feedback === rating ? null : rating);
  const when = turn.completed_at ?? turn.created_at;

  return (
    <div className="reply-actions">
      <button type="button" className="reply-action" onClick={copy} aria-label={copied ? "Copied" : "Copy"} title={copied ? "Copied" : "Copy"}>
        <CopyIcon />
      </button>
      {"speechSynthesis" in window && (
      <button type="button" className={`reply-action ${speaking ? "reply-action-on" : ""}`} onClick={speak} aria-pressed={speaking} aria-label="Read aloud" title="Read aloud">
        <SpeakIcon />
      </button>
      )}
      <button type="button" className={`reply-action ${turn.feedback === "up" ? "reply-action-on" : ""}`} onClick={() => rate("up")} aria-pressed={turn.feedback === "up"} aria-label="Good response" title="Good response">
        <ThumbUpIcon />
      </button>
      <button type="button" className={`reply-action ${turn.feedback === "down" ? "reply-action-on" : ""}`} onClick={() => rate("down")} aria-pressed={turn.feedback === "down"} aria-label="Bad response" title="Bad response">
        <ThumbDownIcon />
      </button>
      <button type="button" className="reply-action" onClick={() => onRegenerate(turn.input)} aria-label="Retry" title="Retry">
        <RetryIcon />
      </button>
      <span className="reply-time muted">{relativeTime(when)}</span>
    </div>
  );
}

/** Answers offered for each approval kind. Each result is the app-server response shape. */
function choicesFor(method: string): { label: string; result: unknown }[] {
  switch (method) {
    case "item/commandExecution/requestApproval":
    case "item/fileChange/requestApproval":
      return [
        { label: "Allow", result: { decision: "accept" } },
        { label: "Deny", result: { decision: "decline" } },
      ];
    case "item/permissions/requestApproval":
      return [{ label: "Deny", result: { permissions: {}, scope: "turn" } }];
    case "item/tool/requestUserInput":
      return [{ label: "Skip", result: { answers: {} } }];
    default:
      return [];
  }
}

function describe(approval: Approval): string {
  const params = approval.params;
  if (typeof params.command === "string") return params.command;
  if (typeof params.reason === "string") return params.reason;
  return approval.method;
}

/** Short labels for what a turn did, in the order it first did them. */
const STEP_VERBS: Record<string, string> = {
  "Web search": "Searching the web",
  Command: "running a command",
  Connector: "using a connector",
  Tool: "using a tool",
  "File change": "changing files",
  Image: "viewing an image",
  "Image generation": "generating an image",
};

/** The collapsed "Thinking" line: what the model has been doing this turn, or just "Thinking". */
function activitySummary(activity: Activity, running: boolean): string {
  const verbs = [...new Set(activity.steps.map((step) => STEP_VERBS[step.kind] ?? step.kind))];
  if (verbs.length === 0) return running ? "Thinking" : "Thought process";
  const text = verbs.join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** A file sent with a message. Clicking the name opens it in the side panel; the buttons download or delete it. */
function FileChip({ id, name, contentType }: { id: string; name: string; contentType: string }) {
  const { open } = usePreview();
  const [gone, setGone] = useState(false);
  const [busy, setBusy] = useState(false);
  if (gone) return null;

  const download = async () => {
    setBusy(true);
    try {
      const bytes = await attachmentApi.download(id);
      const url = URL.createObjectURL(new Blob([bytes], { type: contentType || "application/octet-stream" }));
      const link = document.createElement("a");
      link.href = url;
      link.download = name;
      link.click();
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (err) {
      window.alert(err instanceof Error ? err.message : "Could not download the file.");
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!window.confirm(`Delete “${name}”? It will be removed from this chat.`)) return;
    setBusy(true);
    try {
      await attachmentApi.remove(id);
      setGone(true);
    } catch (err) {
      setBusy(false);
      window.alert(err instanceof Error ? err.message : "Could not delete the file.");
    }
  };

  return (
    <span className="file-chip-wrap">
      <button
        type="button"
        className="file-chip"
        onClick={() => open({ kind: "file", title: name, attachmentId: id, contentType })}
        title={name}
      >
        <span className="file-chip-name">{name}</span>
      </button>
      <button type="button" className="file-chip-action" onClick={() => void download()} disabled={busy} aria-label={`Download ${name}`} title="Download">
        ↓
      </button>
      <button type="button" className="file-chip-action file-chip-danger" onClick={() => void remove()} disabled={busy} aria-label={`Delete ${name}`} title="Delete">
        ×
      </button>
    </span>
  );
}

/** A shell command run during a turn. Collapsed by default, like a tool call in a chat log. */
function CommandStep({ command }: { command: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="command-step">
      <button type="button" className="command-toggle" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span>Bash</span>
        <ChevronIcon className={open ? "thinking-chevron thinking-chevron-open" : "thinking-chevron"} />
      </button>
      {open && (
        <pre className="command-body">
          <code>{command}</code>
        </pre>
      )}
    </div>
  );
}

/** Collapsible record of how a reply was produced: the reasoning and each step taken. */
function ThinkingBlock({ activity, running }: { activity: Activity; running: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="thinking">
      <button type="button" className="thinking-toggle" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span className={running ? "thinking-dots thinking-dots-live" : "thinking-dots"} aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
        <span>{activitySummary(activity, running)}</span>
        <ChevronIcon className={open ? "thinking-chevron thinking-chevron-open" : "thinking-chevron"} />
      </button>
      {open && (
        <div className="thinking-body">
          {activity.reasoning && <div className="thinking-reasoning">{activity.reasoning}</div>}
          {activity.steps.length > 0 && (
            <ul className="thinking-steps">
              {activity.steps.map((step) =>
                step.kind === "Command" ? (
                  <li key={step.id}>
                    <CommandStep command={step.label} />
                  </li>
                ) : (
                  <li key={step.id}>
                    <span className="muted">{step.kind}</span> {step.label}
                  </li>
                ),
              )}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

interface ConversationProps {
  turns: Turn[];
  agentText: Record<string, string>;
  activity: Record<string, Activity>;
  approvals: Approval[];
  onAnswer: (approval: Approval, result: unknown) => void;
  onFeedback: (turnId: string, rating: "up" | "down" | null) => void;
  onRegenerate: (input: string) => void;
}

export function Conversation({ turns, agentText, activity, approvals, onAnswer, onFeedback, onRegenerate }: ConversationProps) {
  return (
    <div className="conversation">
      {turns.map((turn) => {
        // Live text while the turn streams; the saved reply after a reload.
        const reply = (turn.codex_turn_id ? agentText[turn.codex_turn_id] : undefined) || turn.reply || undefined;
        const live = turn.codex_turn_id ? activity[turn.codex_turn_id] : undefined;
        const running = turn.status === "running" || turn.status === "queued";
        return (
          <div className="turn" key={turn.id}>
            <div className="bubble user">{turn.input}</div>
            {turn.attachments && turn.attachments.length > 0 && (
              <div className="turn-files">
                {turn.attachments.map((file) => (
                  <FileChip key={file.id} id={file.id} name={file.filename} contentType={file.content_type} />
                ))}
              </div>
            )}
            {live && (live.reasoning || live.steps.length > 0) && <ThinkingBlock activity={live} running={running} />}
            {reply && (
              <>
                <div className="bubble agent">
                  <MarkdownView source={reply} />
                </div>
                {turn.status === "completed" && <ReplyActions turn={turn} reply={reply} onFeedback={onFeedback} onRegenerate={onRegenerate} />}
              </>
            )}
            <div className={`turn-status status-${turn.status}`}>
              {turn.status === "completed" ? "Done" : turn.status === "running" || turn.status === "queued" ? null : turn.status}
              {turn.error && <span className="turn-error"> · {turn.error}</span>}
            </div>
          </div>
        );
      })}
      {approvals.map((approval) => (
        <div className="approval" key={approval.requestId}>
          <div className="approval-title">Approval needed</div>
          <code className="approval-body">{describe(approval)}</code>
          <div className="approval-actions">
            {choicesFor(approval.method).map((choice) => (
              <button key={choice.label} onClick={() => onAnswer(approval, choice.result)}>
                {choice.label}
              </button>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
