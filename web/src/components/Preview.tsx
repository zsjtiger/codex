import { createContext, useContext, useState } from "react";
import { MarkdownView, highlightCode } from "./Markdown";
import { CopyIcon } from "./icons";

/** What the side panel shows: a code block from a reply, or a document artifact. */
export type PreviewItem =
  | { kind: "code"; title: string; lang: string; text: string }
  | { kind: "document"; title: string; content: string };

interface PreviewContextValue {
  open: (item: PreviewItem) => void;
}

const PreviewContext = createContext<PreviewContextValue>({ open: () => {} });

/** Opens an item in the side panel. Outside a provider, opening does nothing. */
export const usePreview = () => useContext(PreviewContext);

/** Holds the open preview and renders the side panel. Wrap the app in it. */
export function PreviewProvider({ children }: { children: React.ReactNode }) {
  const [item, setItem] = useState<PreviewItem | null>(null);
  return (
    <PreviewContext.Provider value={{ open: setItem }}>
      {children}
      {item && <PreviewPanel item={item} onClose={() => setItem(null)} />}
    </PreviewContext.Provider>
  );
}

function PreviewPanel({ item, onClose }: { item: PreviewItem; onClose: () => void }) {
  const [copied, setCopied] = useState(false);
  const text = item.kind === "code" ? item.text : item.content;
  const label = item.kind === "code" ? item.lang || "code" : "document";
  const highlighted = item.kind === "code" ? highlightCode(item.text, item.lang) : null;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard access can be refused; the text stays selectable.
    }
  };

  return (
    <aside className="preview-panel" role="dialog" aria-label={item.title}>
      <header className="preview-header">
        <div className="preview-title">
          <span className="preview-name">{item.title}</span>
          <span className="muted small">{label}</span>
        </div>
        <button type="button" className="icon-button" onClick={copy} aria-label={copied ? "Copied" : "Copy"} title={copied ? "Copied" : "Copy"}>
          <CopyIcon />
        </button>
        <button type="button" className="banner-close" onClick={onClose} aria-label="Close">
          ×
        </button>
      </header>
      <div className="preview-body">
        {item.kind === "document" ? (
          <MarkdownView source={item.content} />
        ) : highlighted?.known ? (
          <pre className="code-block">
            <code dangerouslySetInnerHTML={{ __html: highlighted.html }} />
          </pre>
        ) : (
          <pre>
            <code>{item.text}</code>
          </pre>
        )}
      </div>
    </aside>
  );
}
