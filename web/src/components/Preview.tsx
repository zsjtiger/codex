import { createContext, useContext, useEffect, useRef, useState } from "react";
import * as mammoth from "mammoth";
import { artifacts as artifactApi, attachments as attachmentApi } from "../api/resources";
import type { ArtifactVersion } from "../api/types";
import { MarkdownView, highlightCode } from "./Markdown";
import { CopyIcon } from "./icons";
import { PdfView } from "./PdfView";
import { htmlToMarkdown, markdownToHtml } from "./richtext";
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
  // The earlier versions of a document, and the one being read, when the history is open.
  const [history, setHistory] = useState<{ versions: ArtifactVersion[] | null; selected: ArtifactVersion | null } | null>(null);

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

  const openHistory = async () => {
    if (item.kind !== "document" || !item.artifactId) return;
    setHistory({ versions: null, selected: null });
    try {
      const versions = await artifactApi.versions(item.artifactId);
      setHistory({ versions, selected: null });
    } catch (err) {
      setHistory(null);
      window.alert(err instanceof Error ? err.message : tx("Could not load the history."));
    }
  };

  // Restoring saves the old text as a new edit, so the version being replaced is kept too.
  const restore = async (version: ArtifactVersion) => {
    if (item.kind !== "document" || !item.artifactId) return;
    setSaving(true);
    try {
      const saved = { title: version.title, content: version.content };
      await artifactApi.update(item.artifactId, saved);
      setDoc(saved);
      setHistory(null);
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
        {editing && <span className="doc-status">{tx("Editing")}</span>}
        {editable && !editing && (
          <button type="button" className="link-button" onClick={history ? () => setHistory(null) : openHistory}>
            {history ? tx("Done") : tx("History")}
          </button>
        )}
        {editable && !editing && !history && (
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
        ) : history ? (
          <DocumentHistory history={history} onSelect={(selected) => setHistory({ ...history, selected })} onBack={() => setHistory({ ...history, selected: null })} onRestore={restore} busy={saving} />
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
            <RichEditor initial={draft.content} onChange={(content) => setDraft((current) => ({ ...current, content }))} />
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

/** Toolbar buttons for the editor: each runs a formatting command on the selection. */
const RICH_TOOLS: { label: string; text: string; run: (command: (name: string, value?: string) => void) => void }[] = [
  { label: "Bold", text: "B", run: (command) => command("bold") },
  { label: "Italic", text: "I", run: (command) => command("italic") },
  { label: "Heading 2", text: "H2", run: (command) => command("formatBlock", "h2") },
  { label: "Heading 3", text: "H3", run: (command) => command("formatBlock", "h3") },
  { label: "Bullet list", text: "•", run: (command) => command("insertUnorderedList") },
  { label: "Numbered list", text: "1.", run: (command) => command("insertOrderedList") },
  { label: "Quote", text: "“”", run: (command) => command("formatBlock", "blockquote") },
  { label: "Code block", text: "</>", run: (command) => command("formatBlock", "pre") },
  {
    label: "Link",
    text: "🔗",
    run: (command) => {
      const url = window.prompt(tx("Link address"));
      if (url && /^https?:\/\//.test(url)) command("createLink", url);
    },
  },
];

/**
 * A rich-text editor for a document. The text is edited as formatted text; it is reported back as
 * Markdown, the form the document is stored in. The page is set once, so typing keeps the caret.
 */
function RichEditor({ initial, onChange }: { initial: string; onChange: (markdown: string) => void }) {
  const area = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (area.current) area.current.innerHTML = markdownToHtml(initial);
    // Only the first Markdown seeds the page; after that the page is the source of the text.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const report = () => onChange(htmlToMarkdown(area.current?.innerHTML ?? ""));
  const command = (name: string, value?: string) => {
    area.current?.focus();
    document.execCommand(name, false, value);
    report();
  };

  return (
    <div className="rich-editor">
      <div className="rich-toolbar" role="toolbar" aria-label={tx("Formatting")}>
        {RICH_TOOLS.map((tool) => (
          <button
            key={tool.label}
            type="button"
            className="rich-tool"
            // Keeps the selection in the text while a tool is pressed.
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => tool.run(command)}
            aria-label={tx(tool.label)}
            title={tx(tool.label)}
          >
            {tool.text}
          </button>
        ))}
      </div>
      <div
        ref={area}
        className="rich-content"
        contentEditable
        suppressContentEditableWarning
        role="textbox"
        aria-multiline="true"
        aria-label={tx("Content")}
        onInput={report}
      />
    </div>
  );
}

/** The earlier versions of a document, newest first; one can be read and restored. */
function DocumentHistory({
  history,
  onSelect,
  onBack,
  onRestore,
  busy,
}: {
  history: { versions: ArtifactVersion[] | null; selected: ArtifactVersion | null };
  onSelect: (version: ArtifactVersion) => void;
  onBack: () => void;
  onRestore: (version: ArtifactVersion) => void;
  busy: boolean;
}) {
  if (history.versions === null) return <p className="muted">{tx("Loading…")}</p>;
  const { selected } = history;
  if (selected) {
    return (
      <div className="doc-history">
        <div className="doc-history-bar">
          <button type="button" className="link-button" onClick={onBack}>
            {tx("Back")}
          </button>
          <span className="muted small">{new Date(selected.replaced_at).toLocaleString()}</span>
          <button type="button" className="primary" onClick={() => onRestore(selected)} disabled={busy}>
            {tx("Restore")}
          </button>
        </div>
        <p className="muted small">{tx("A version that was replaced by a later edit.")}</p>
        <MarkdownView source={selected.content} />
      </div>
    );
  }
  if (history.versions.length === 0) return <p className="muted">{tx("No earlier versions.")}</p>;
  return (
    <ul className="doc-history">
      {history.versions.map((version) => (
        <li key={version.id}>
          <button type="button" className="doc-history-item" onClick={() => onSelect(version)}>
            <strong>{version.title}</strong>
            <span className="muted small">{new Date(version.replaced_at).toLocaleString()}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

function contentLabel(contentType: string, title: string): string {
  const kind = fileKind(contentType, title);
  return tx({ pdf: "PDF", word: "Word", slides: "PowerPoint", sheet: "Excel", markdown: "Markdown", text: "text", other: "File" }[kind]);
}
