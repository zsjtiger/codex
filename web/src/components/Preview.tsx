import { createContext, useContext, useEffect, useState } from "react";
import * as mammoth from "mammoth";
import { artifacts as artifactApi, attachments as attachmentApi } from "../api/resources";
import { MarkdownView, highlightCode } from "./Markdown";
import { CopyIcon } from "./icons";
import { PdfView } from "./PdfView";
import { tx } from "../i18n";

/** What the side panel shows: a code block, a document artifact, or a file sent in a message. */
export type PreviewItem =
  | { kind: "code"; title: string; lang: string; text: string }
  | { kind: "document"; title: string; content: string; artifactId?: string; onSaved?: (doc: { title: string; content: string }) => void }
  | { kind: "file"; title: string; attachmentId: string; contentType: string };

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

/** mammoth ships typings without its Markdown converter, which the library provides at runtime. */
function convertWordToMarkdown(bytes: ArrayBuffer): Promise<string> {
  const convert = (mammoth as unknown as { convertToMarkdown: (input: { arrayBuffer: ArrayBuffer }) => Promise<{ value: string }> })
    .convertToMarkdown;
  return convert({ arrayBuffer: bytes }).then((result) => result.value);
}

/** The kinds of file the panel can show, from the stored content type and name. */
type FileKind = "pdf" | "word" | "slides" | "sheet" | "markdown" | "text" | "other";

function fileKind(contentType: string, title: string): FileKind {
  const name = title.toLowerCase();
  if (contentType === "application/pdf" || name.endsWith(".pdf")) return "pdf";
  if (contentType.includes("presentationml") || name.endsWith(".pptx")) return "slides";
  if (contentType.includes("spreadsheetml") || name.endsWith(".xlsx")) return "sheet";
  if (contentType.includes("wordprocessingml") || name.endsWith(".docx")) return "word";
  if (contentType === "text/markdown" || name.endsWith(".md") || name.endsWith(".markdown")) return "markdown";
  if (contentType.startsWith("text/") || contentType === "application/json") return "text";
  return "other";
}

/** Loads an uploaded file and shows it: PDF in the browser's viewer, Word as Markdown, text as text. */
function FilePreview({ attachmentId, title, contentType }: { attachmentId: string; title: string; contentType: string }) {
  const [state, setState] = useState<{ status: "loading" } | { status: "error"; message: string } | { status: "ready"; view: ReactViewState }>({
    status: "loading",
  });

  useEffect(() => {
    let current = true;
    setState({ status: "loading" });
    const kind = fileKind(contentType, title);
    // Slides and sheets are converted to PDF by the gateway; the original bytes are not needed.
    const load = kind === "slides" || kind === "sheet" ? attachmentApi.previewPdf(attachmentId) : attachmentApi.download(attachmentId);
    load
      .then(async (bytes) => {
        let view: ReactViewState;
        if (kind === "pdf" || kind === "slides" || kind === "sheet") {
          view = { kind: "pdf", data: bytes };
        } else if (kind === "word") {
          const result = await convertWordToMarkdown(bytes);
          view = { kind, markdown: result };
        } else {
          const text = new TextDecoder().decode(bytes);
          view = { kind: kind === "markdown" ? "markdown" : "text", text };
        }
        if (current) setState({ status: "ready", view });
      })
      .catch((err) => {
        if (current) setState({ status: "error", message: err instanceof Error ? err.message : tx("Could not open the file.") });
      });
    return () => {
      current = false;
    };
  }, [attachmentId, contentType, title]);

  if (state.status === "loading") return <p className="muted">{tx("Loading…")}</p>;
  if (state.status === "error") return <p className="banner error">{state.message}</p>;
  const { view } = state;
  if (view.kind === "pdf") return <PdfView data={view.data} />;
  if (view.kind === "word") return <MarkdownView source={view.markdown} />;
  if (view.kind === "markdown") return <MarkdownView source={view.text} />;
  if (view.kind === "text") return <pre className="preview-text">{view.text}</pre>;
  return <p className="muted">{tx("This file type cannot be previewed.")}</p>;
}

type ReactViewState =
  | { kind: "pdf"; data: ArrayBuffer }
  | { kind: "word"; markdown: string }
  | { kind: "markdown"; text: string }
  | { kind: "text"; text: string }
  | { kind: "other" };

function PreviewPanel({ item, onClose }: { item: PreviewItem; onClose: () => void }) {
  const [copied, setCopied] = useState(false);
  // A document keeps its own copy, so an edit shows at once and survives the panel being redrawn.
  const [doc, setDoc] = useState(() => (item.kind === "document" ? { title: item.title, content: item.content } : null));
  const [editing, setEditing] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [draft, setDraft] = useState({ title: "", content: "" });
  const [saving, setSaving] = useState(false);

  const text = item.kind === "code" ? item.text : doc ? doc.content : null;
  const label = item.kind === "code" ? item.lang || "code" : item.kind === "document" ? tx("Document") : contentLabel(item.contentType, item.title);
  const highlighted = item.kind === "code" ? highlightCode(item.text, item.lang) : null;
  const title = doc ? doc.title : item.title;
  const editable = item.kind === "document" && Boolean(item.artifactId);

  const copy = async () => {
    if (text === null) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard access can be refused; the text stays selectable.
    }
  };

  const startEditing = () => {
    if (!doc) return;
    setDraft(doc);
    setEditing(true);
  };

  const save = async () => {
    if (item.kind !== "document" || !item.artifactId) return;
    setSaving(true);
    try {
      const saved = { title: draft.title.trim() || doc?.title || tx("Document"), content: draft.content };
      await artifactApi.update(item.artifactId, saved);
      setDoc(saved);
      setEditing(false);
      item.onSaved?.(saved);
    } catch (err) {
      window.alert(err instanceof Error ? err.message : tx("Could not save the document."));
    } finally {
      setSaving(false);
    }
  };

  return (
    <aside className={`preview-panel ${expanded ? "preview-panel-wide" : ""}`} role="dialog" aria-label={title}>
      <header className="preview-header">
        <div className="preview-title">
          <span className="preview-name">{title}</span>
          <span className="muted small">{label}</span>
        </div>
        {editable && !editing && (
          <button type="button" className="link-button" onClick={startEditing}>
            {tx("Edit")}
          </button>
        )}
        {item.kind === "document" && (
          <button type="button" className="icon-button" onClick={() => setExpanded((open) => !open)} aria-label={expanded ? tx("Collapse") : tx("Expand")} title={expanded ? tx("Collapse") : tx("Expand")}>
            {expanded ? "⤡" : "⤢"}
          </button>
        )}
        {text !== null && (
          <button type="button" className="icon-button" onClick={copy} aria-label={copied ? "Copied" : "Copy"} title={copied ? "Copied" : "Copy"}>
            <CopyIcon />
          </button>
        )}
        <button type="button" className="banner-close" onClick={onClose} aria-label={tx("Close")}>
          ×
        </button>
      </header>
      <div className="preview-body">
        {item.kind === "file" ? (
          <FilePreview attachmentId={item.attachmentId} title={item.title} contentType={item.contentType} />
        ) : item.kind === "document" && editing ? (
          <form
            className="doc-editor"
            onSubmit={(event) => {
              event.preventDefault();
              void save();
            }}
          >
            <input
              className="doc-title-input"
              value={draft.title}
              onChange={(event) => setDraft({ ...draft, title: event.target.value })}
              aria-label={tx("Title")}
            />
            <textarea
              className="doc-content-input"
              value={draft.content}
              onChange={(event) => setDraft({ ...draft, content: event.target.value })}
              aria-label={tx("Content")}
            />
            <div className="doc-editor-actions">
              <button type="button" className="link-button" onClick={() => setEditing(false)} disabled={saving}>
                {tx("Cancel")}
              </button>
              <button className="primary" disabled={saving}>
                {saving ? tx("Saving…") : tx("Save")}
              </button>
            </div>
          </form>
        ) : item.kind === "document" ? (
          <MarkdownView source={doc?.content ?? ""} />
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

function contentLabel(contentType: string, title: string): string {
  const kind = fileKind(contentType, title);
  return tx({ pdf: "PDF", word: "Word", slides: "PowerPoint", sheet: "Excel", markdown: "Markdown", text: "text", other: "File" }[kind]);
}
