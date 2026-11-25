import { useState } from "react";
import type { Approval } from "../state/useCodex";
import { tx } from "../i18n";

/** The part of an MCP form schema this card draws: one input per property. */
interface FieldSchema {
  type?: string;
  title?: string;
  enum?: unknown[];
  oneOf?: { const: unknown; title?: string }[];
}

interface ElicitationParams {
  serverName?: string;
  message?: string;
  requestedSchema?: { properties?: Record<string, FieldSchema>; required?: string[] };
}

/**
 * An MCP server asks the person for input with a form. Answering accepts with the values as the content,
 * or declines or cancels. Required fields must be filled before accepting.
 */
export function ElicitationCard({ approval, onAnswer }: { approval: Approval; onAnswer: (result: unknown) => void }) {
  const params = approval.params as ElicitationParams;
  const fields = Object.entries(params.requestedSchema?.properties ?? {});
  const required = new Set(params.requestedSchema?.required ?? []);
  const [values, setValues] = useState<Record<string, unknown>>({});

  const set = (key: string, value: unknown) => setValues((prev) => ({ ...prev, [key]: value }));
  const filled = (key: string) => values[key] !== undefined && values[key] !== "";
  const missing = fields.some(([key]) => required.has(key) && !filled(key));

  const accept = () => {
    const content = Object.fromEntries(Object.entries(values).filter(([key]) => filled(key)));
    onAnswer({ action: "accept", content });
  };

  return (
    <div className="approval elicitation">
      <div className="approval-title">{tx("A tool needs your answer")}</div>
      {params.serverName && <div className="muted small">{params.serverName}</div>}
      {params.message && <div className="elicitation-message">{params.message}</div>}
      {fields.map(([key, field]) => {
        const label = field.title ?? key;
        const options = field.oneOf ? field.oneOf.map((option) => option.const) : field.enum;
        const optionLabel = (value: unknown) => field.oneOf?.find((option) => option.const === value)?.title ?? String(value);
        return (
          <label key={key} className="elicitation-field">
            <span>
              {label}
              {required.has(key) && " *"}
            </span>
            {options ? (
              <select value={values[key] === undefined ? "" : String(values[key])} onChange={(event) => {
                const chosen = options.find((option) => String(option) === event.target.value);
                set(key, chosen ?? "");
              }}>
                <option value="" />
                {options.map((option) => (
                  <option key={String(option)} value={String(option)}>
                    {optionLabel(option)}
                  </option>
                ))}
              </select>
            ) : field.type === "boolean" ? (
              <input type="checkbox" checked={values[key] === true} onChange={(event) => set(key, event.target.checked)} />
            ) : field.type === "number" || field.type === "integer" ? (
              <input type="number" value={values[key] === undefined ? "" : String(values[key])} onChange={(event) => set(key, event.target.value === "" ? "" : Number(event.target.value))} />
            ) : (
              <input type="text" value={values[key] === undefined ? "" : String(values[key])} onChange={(event) => set(key, event.target.value)} />
            )}
          </label>
        );
      })}
      <div className="approval-actions">
        <button disabled={missing} onClick={accept}>
          {tx("Submit")}
        </button>
        <button onClick={() => onAnswer({ action: "decline" })}>{tx("Decline")}</button>
        <button onClick={() => onAnswer({ action: "cancel" })}>{tx("Cancel")}</button>
      </div>
    </div>
  );
}
