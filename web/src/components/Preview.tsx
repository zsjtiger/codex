import { createContext, useContext, useEffect, useState } from "react";
import * as mammoth from "mammoth";
import { attachments as attachmentApi } from "../api/resources";
import { MarkdownView, highlightCode } from "./Markdown";
import { CopyIcon } from "./icons";

/** What the side panel shows: a code block, a document artifact, or a file sent in a message. */
export type PreviewItem =
  | { kind: "code"; title: string; lang: string; text: string }
  | { kind: "document"; title: string; content: string }
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
    let objectUrl: string | null = null;
    setState({ status: "loading" });
    const kind = fileKind(contentType, title);
    // Slides and sheets are converted to PDF by the gateway; the original bytes are not needed.
    const load = kind === "slides" || kind === "sheet" ? attachmentApi.previewPdf(attachmentId) : attachmentApi.download(attachmentId);
    load
      .then(async (bytes) => {
        let view: ReactViewState;
        if (kind === "pdf" || kind === "slides" || kind === "sheet") {
          objectUrl = URL.createObjectURL(new Blob([bytes], { type: "application/pdf" }));
          view = { kind: "pdf", url: objectUrl };
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
        if (current) setState({ status: "error", message: err instanceof Error ? err.message : "Could not open the file." });
      });
    return () => {
      current = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [attachmentId, contentType, title]);

  if (state.status === "loading") return <p className="muted">Loading…</p>;
  if (state.status === "error") return <p className="banner error">{state.message}</p>;
  const { view } = state;
  if (view.kind === "pdf") return <iframe className="preview-frame" src={view.url} title={title} />;
  if (view.kind === "word") return <MarkdownView source={view.markdown} />;
  if (view.kind === "markdown") return <MarkdownView source={view.text} />;
  if (view.kind === "text") return <pre className="preview-text">{view.text}</pre>;
  return <p className="muted">This file type cannot be previewed.</p>;
}

type ReactViewState =
  | { kind: "pdf"; url: string }
  | { kind: "word"; markdown: string }
  | { kind: "markdown"; text: string }
  | { kind: "text"; text: string }
  | { kind: "other" };

function PreviewPanel({ item, onClose }: { item: PreviewItem; onClose: () => void }) {
  const [copied, setCopied] = useState(false);
  const text = item.kind === "code" ? item.text : item.kind === "document" ? item.content : null;
  const label = item.kind === "code" ? item.lang || "code" : item.kind === "document" ? "document" : contentLabel(item.contentType, item.title);
  const highlighted = item.kind === "code" ? highlightCode(item.text, item.lang) : null;

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

  return (
    <aside className="preview-panel" role="dialog" aria-label={item.title}>
      <header className="preview-header">
        <div className="preview-title">
          <span className="preview-name">{item.title}</span>
          <span className="muted small">{label}</span>
        </div>
        {text !== null && (
          <button type="button" className="icon-button" onClick={copy} aria-label={copied ? "Copied" : "Copy"} title={copied ? "Copied" : "Copy"}>
            <CopyIcon />
          </button>
        )}
        <button type="button" className="banner-close" onClick={onClose} aria-label="Close">
          ×
        </button>
      </header>
      <div className="preview-body">
        {item.kind === "file" ? (
          <FilePreview attachmentId={item.attachmentId} title={item.title} contentType={item.contentType} />
        ) : item.kind === "document" ? (
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

function contentLabel(contentType: string, title: string): string {
  const kind = fileKind(contentType, title);
  return { pdf: "PDF", word: "Word", slides: "PowerPoint", sheet: "Excel", markdown: "Markdown", text: "text", other: "file" }[kind];
}
