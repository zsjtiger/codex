import { useEffect, useRef, useState } from "react";
import { MarkdownView } from "./Markdown";
import { CopyIcon, RetryIcon, SpeakIcon, ThumbDownIcon, ThumbUpIcon } from "./icons";
import type { Activity, Approval } from "../state/useCodex";
import { ChevronIcon } from "./icons";
import type { ToolUse } from "../state/toolUses";
import type { WorkFile, WorkItem, WorkLog } from "../state/work";
import { usePreview } from "./Preview";
import { attachments as attachmentApi } from "../api/resources";
import { isDesktop, saveFileAs, showInFolder } from "../desktop";
import type { Turn } from "../api/types";
import { tx } from "../i18n";

/** "just now", "5 min ago", "3 h ago", or the date for older replies. */
function relativeTime(iso: string): string {
  const seconds = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (seconds < 60) return tx("just now");
  if (seconds < 3600) return `${Math.floor(seconds / 60)} ${tx("min ago")}`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} ${tx("h ago")}`;
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
      <button type="button" className={`reply-action ${speaking ? "reply-action-on" : ""}`} onClick={speak} aria-pressed={speaking} aria-label={tx("Read aloud")} title={tx("Read aloud")}>
        <SpeakIcon />
      </button>
      )}
      <button type="button" className={`reply-action ${turn.feedback === "up" ? "reply-action-on" : ""}`} onClick={() => rate("up")} aria-pressed={turn.feedback === "up"} aria-label={tx("Good response")} title={tx("Good response")}>
        <ThumbUpIcon />
      </button>
      <button type="button" className={`reply-action ${turn.feedback === "down" ? "reply-action-on" : ""}`} onClick={() => rate("down")} aria-pressed={turn.feedback === "down"} aria-label={tx("Bad response")} title={tx("Bad response")}>
        <ThumbDownIcon />
      </button>
      <button type="button" className="reply-action" onClick={() => onRegenerate(turn.input)} aria-label={tx("Retry")} title={tx("Retry")}>
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
        { label: tx("Allow"), result: { decision: "accept" } },
        { label: tx("Deny"), result: { decision: "decline" } },
      ];
    case "item/permissions/requestApproval":
      return [{ label: tx("Deny"), result: { permissions: {}, scope: "turn" } }];
    case "item/tool/requestUserInput":
      return [{ label: tx("Skip"), result: { answers: {} } }];
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
  const verbs = [...new Set(activity.steps.map((step) => tx(STEP_VERBS[step.kind] ?? step.kind)))];
  if (verbs.length === 0) return running ? tx("Thinking") : tx("Thought process");
  const text = verbs.join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** A file sent with a message. Clicking the name opens it in the side panel; the buttons download or delete it. */
function FileChip({ id, name, contentType }: { id: string; name: string; contentType: string }) {
  const { open } = usePreview();
  const [gone, setGone] = useState(false);
  const [busy, setBusy] = useState(false);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  if (gone) return null;

  const download = async () => {
    setBusy(true);
    try {
      const bytes = await attachmentApi.download(id);
      if (isDesktop()) {
        // The desktop app asks where to save, then can show the file in its folder.
        const path = await saveFileAs(name, new Uint8Array(bytes));
        if (path) setSavedPath(path);
        return;
      }
      const url = URL.createObjectURL(new Blob([bytes], { type: contentType || "application/octet-stream" }));
      const link = document.createElement("a");
      link.href = url;
      link.download = name;
      link.click();
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (err) {
      window.alert(err instanceof Error ? err.message : tx("Could not download the file."));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!window.confirm(`${tx("Delete")} “${name}”? ${tx("It will be removed from this chat.")}`)) return;
    setBusy(true);
    try {
      await attachmentApi.remove(id);
      setGone(true);
    } catch (err) {
      setBusy(false);
      window.alert(err instanceof Error ? err.message : tx("Could not delete the file."));
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
      <button type="button" className="file-chip-action" onClick={() => void download()} disabled={busy} aria-label={`Download ${name}`} title={tx("Download")}>
        ↓
      </button>
      <button type="button" className="file-chip-action file-chip-danger" onClick={() => void remove()} disabled={busy} aria-label={`Delete ${name}`} title={tx("Delete")}>
        ×
      </button>
      {savedPath && (
        <button type="button" className="file-chip-action" onClick={() => void showInFolder(savedPath)}>
          {tx("Show in folder")}
        </button>
      )}
    </span>
  );
}

/** A shell command run during a turn. Collapsed by default, like a tool call in a chat log. */
function CommandStep({ command }: { command: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="command-step">
      <button type="button" className="command-toggle" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span>{tx("Bash")}</span>
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
                    <span className="muted">{tx(step.kind)}</span> {step.label}
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

const FILE_VERB: Record<string, string> = { add: "Added", delete: "Deleted", update: "Edited" };

/** Added and removed line counts of a unified diff. */
function diffCounts(diff: string): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) added++;
    else if (line.startsWith("-") && !line.startsWith("---")) removed++;
  }
  return { added, removed };
}

function diffLineClass(line: string): string {
  if (line.startsWith("@@")) return "diff-hunk";
  if (line.startsWith("+") && !line.startsWith("+++")) return "diff-add-line";
  if (line.startsWith("-") && !line.startsWith("---")) return "diff-del-line";
  return "";
}

function StatusMark({ status }: { status: string }) {
  const label =
    status === "completed" ? tx("Done")
    : status === "failed" ? tx("Failed")
    : status === "declined" ? tx("Declined")
    : tx("Running…");
  return <span className={`work-status work-status-${status}`} role="img" aria-label={label} title={label} />;
}

/** A shell command. Open while it runs so its output streams in; the exit code shows when it ends. */
function CommandRow({ item }: { item: WorkItem }) {
  const running = item.status === "inProgress";
  const [open, setOpen] = useState(true);
  return (
    <div className="work-item">
      <button type="button" className="work-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        <StatusMark status={item.status} />
        <code className="work-command">$ {item.command}</code>
        <ChevronIcon className={open ? "thinking-chevron thinking-chevron-open" : "thinking-chevron"} />
      </button>
      {open && (
        <>
          {item.output && <pre className="work-output">{item.output}</pre>}
          {!running && item.exitCode !== null && (
            <div className="work-foot muted small">
              {tx("exit")} {item.exitCode}
              {item.durationMs !== null ? ` · ${(item.durationMs / 1000).toFixed(1)}s` : ""}
            </div>
          )}
        </>
      )}
    </div>
  );
}

/** One changed file. Expanding shows its diff, with added and removed lines coloured. */
function FileChangeRow({ file, status }: { file: WorkFile; status: string }) {
  const [open, setOpen] = useState(false);
  const counts = diffCounts(file.diff);
  return (
    <div className="work-item">
      <button type="button" className="work-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        <StatusMark status={status} />
        <span className="work-verb">{tx(FILE_VERB[file.kind] ?? "Edited")}</span>
        <code className="work-path">{file.path}</code>
        <span className="work-counts">
          <span className="diff-add-count">+{counts.added}</span> <span className="diff-del-count">−{counts.removed}</span>
        </span>
        <ChevronIcon className={open ? "thinking-chevron thinking-chevron-open" : "thinking-chevron"} />
      </button>
      {open && (
        <pre className="work-diff">
          {file.diff
            ? file.diff.split("\n").map((line, index) => (
                <span key={index} className={diffLineClass(line)}>
                  {line}
                  {"\n"}
                </span>
              ))
            : tx("No text diff.")}
        </pre>
      )}
    </div>
  );
}

/** The plan Codex is working from, with each step's state. */
function PlanView({ plan }: { plan: NonNullable<WorkLog["plan"]> }) {
  const mark = (status: string) => (status === "completed" ? "✓" : status === "inProgress" ? "◐" : "○");
  return (
    <div className="work-plan">
      <div className="muted small">{tx("Plan")}</div>
      {plan.explanation && <div className="muted small">{plan.explanation}</div>}
      <ul>
        {plan.steps.map((step, index) => (
          <li key={index} className={`plan-${step.status}`}>
            <span className="plan-mark" aria-hidden="true">{mark(step.status)}</span>
            <span>{step.step}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Everything Codex has done in a turn so far, in order: commands, file changes and the plan. */
function WorkLogView({ log }: { log: WorkLog }) {
  return (
    <div className="work-log">
      {log.plan && log.plan.steps.length > 0 && <PlanView plan={log.plan} />}
      {log.order.map((id) => {
        const item = log.items[id];
        if (!item) return null;
        if (item.type === "command") return <CommandRow key={id} item={item} />;
        return item.files.map((file, index) => <FileChangeRow key={`${id}-${index}`} file={file} status={item.status} />);
      })}
    </div>
  );
}

type StepGroup = { kind: string; steps: Activity["steps"] };

/** Steps of the same kind, in the order they first happened. */
function groupSteps(steps: Activity["steps"]): StepGroup[] {
  const groups = new Map<string, StepGroup>();
  for (const step of steps) {
    const group = groups.get(step.kind) ?? { kind: step.kind, steps: [] };
    group.steps.push(step);
    groups.set(step.kind, group);
  }
  return [...groups.values()];
}

/** The summary line for a group, e.g. "Ran 3 commands". Each kind has its own verb and noun. */
function groupLabel(group: StepGroup): string {
  const count = group.steps.length;
  const [verb, noun] =
    group.kind === "Command" ? ["Ran", count === 1 ? "command" : "commands"]
    : group.kind === "File change" ? ["Changed", count === 1 ? "file" : "files"]
    : group.kind === "Web search" ? ["Searched", count === 1 ? "time" : "times"]
    : ["Used", count === 1 ? "tool" : "tools"];
  return `${tx(verb)} ${count} ${tx(noun)}`;
}

/** One collapsed row per kind of step, like "Ran 3 commands ⌄". Expanding shows each step. */
function CodeActivityRow({ group }: { group: StepGroup }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="code-activity-row">
      <button type="button" className="code-activity-toggle" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span>{groupLabel(group)}</span>
        <ChevronIcon className={open ? "thinking-chevron thinking-chevron-open" : "thinking-chevron"} />
      </button>
      {open && (
        <ul className="code-activity-body">
          {group.steps.map((step) => (
            <li key={step.id}>
              {group.kind === "Command" ? <code>{step.label}</code> : step.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** What a code turn did, grouped by kind. Shown above the reply, as the reference does. */
function CodeActivity({ activity }: { activity: Activity }) {
  return (
    <div className="code-activity">
      {groupSteps(activity.steps).map((group) => (
        <CodeActivityRow key={group.kind} group={group} />
      ))}
    </div>
  );
}

interface ConversationProps {
  /** "code" shows replies as plain text with grouped activity, as in a code session. */
  variant?: "chat" | "code";
  turns: Turn[];
  agentText: Record<string, string>;
  activity: Record<string, Activity>;
  /** What Codex is doing per turn: commands with live output, file diffs and the plan. */
  work?: Record<string, WorkLog>;
  approvals: Approval[];
  onAnswer: (approval: Approval, result: unknown) => void;
  onFeedback: (turnId: string, rating: "up" | "down" | null) => void;
  onRegenerate: (input: string) => void;
}

export function Conversation({ variant = "chat", turns, agentText, activity, work = {}, approvals, onAnswer, onFeedback, onRegenerate }: ConversationProps) {
  const code = variant === "code";
  return (
    <div className={code ? "conversation code-conversation" : "conversation"}>
      {turns.map((turn) => {
        // Live text while the turn streams; the saved reply after a reload.
        const reply = (turn.codex_turn_id ? agentText[turn.codex_turn_id] : undefined) || turn.reply || undefined;
        const live = turn.codex_turn_id ? activity[turn.codex_turn_id] : undefined;
        const running = turn.status === "running" || turn.status === "queued";
        if (code) {
          const workLog = turn.codex_turn_id ? work[turn.codex_turn_id] : undefined;
          const hasWork = Boolean(workLog && workLog.order.length > 0);
          // Commands and file changes show in the work log; the rest stays as grouped rows.
          const otherSteps = live && {
            ...live,
            steps: hasWork ? live.steps.filter((step) => step.kind !== "Command" && step.kind !== "File change") : live.steps,
          };
          return (
            <div className="turn code-turn" key={turn.id}>
              <div className="code-user">{turn.input}</div>
              {turn.attachments && turn.attachments.length > 0 && (
                <div className="turn-files">
                  {turn.attachments.map((file) => (
                    <FileChip key={file.id} id={file.id} name={file.filename} contentType={file.content_type} />
                  ))}
                </div>
              )}
              {workLog && (workLog.order.length > 0 || workLog.plan) && <WorkLogView log={workLog} />}
              {otherSteps && otherSteps.steps.length > 0 && <CodeActivity activity={otherSteps} />}
              {reply && (
                <>
                  <div className="code-reply">
                    <MarkdownView source={reply} />
                  </div>
                  {turn.status === "completed" && <ReplyActions turn={turn} reply={reply} onFeedback={onFeedback} onRegenerate={onRegenerate} />}
                </>
              )}
              {(turn.status !== "completed" && turn.status !== "running" && turn.status !== "queued") || turn.error ? (
                <div className={`turn-status status-${turn.status}`}>
                  {tx(turn.status)}
                  {turn.error && <span className="turn-error"> · {turn.error}</span>}
                </div>
              ) : null}
            </div>
          );
        }
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
              {turn.status === "completed" ? tx("Done") : turn.status === "running" || turn.status === "queued" ? null : tx(turn.status)}
              {turn.error && <span className="turn-error"> · {turn.error}</span>}
            </div>
          </div>
        );
      })}
      {approvals.map((approval) => (
        <div className="approval" key={approval.requestId}>
          <div className="approval-title">{tx("Approval needed")}</div>
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
