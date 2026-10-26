import type { Approval } from "../state/useCodex";
import type { Turn } from "../api/types";

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

interface ConversationProps {
  turns: Turn[];
  agentText: Record<string, string>;
  approvals: Approval[];
  onAnswer: (approval: Approval, result: unknown) => void;
}

export function Conversation({ turns, agentText, approvals, onAnswer }: ConversationProps) {
  return (
    <div className="conversation">
      {turns.map((turn) => {
        const reply = turn.codex_turn_id ? agentText[turn.codex_turn_id] : undefined;
        return (
          <div className="turn" key={turn.id}>
            <div className="bubble user">{turn.input}</div>
            {reply && <div className="bubble agent">{reply}</div>}
            <div className={`turn-status status-${turn.status}`}>
              {turn.status === "completed" ? "Done" : turn.status}
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
