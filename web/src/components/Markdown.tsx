import katex from "katex";
import "katex/dist/katex.min.css";
import { type ReactNode } from "react";

export interface Heading {
  id: string;
  level: number;
  text: string;
}

type Block =
  | { type: "heading"; level: number; text: string; id: string }
  | { type: "paragraph"; text: string }
  | { type: "list"; ordered: boolean; items: string[] }
  | { type: "code"; lang: string; text: string }
  | { type: "math"; tex: string }
  | { type: "quote"; text: string }
  | { type: "table"; header: string[]; align: TableAlign[]; rows: string[][] }
  | { type: "rule" };

type TableAlign = "left" | "center" | "right" | null;

/** Splits a GFM table row into trimmed cells, ignoring the optional outer pipes. */
function splitRow(line: string): string[] {
  let row = line.trim();
  if (row.startsWith("|")) row = row.slice(1);
  if (row.endsWith("|") && !row.endsWith("\\|")) row = row.slice(0, -1);
  return row.split(/(?<!\\)\|/).map((cell) => cell.trim().replace(/\\\|/g, "|"));
}

const TABLE_DIVIDER = /^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?\s*$/;

function parseAlign(divider: string): TableAlign[] {
  return splitRow(divider).map((cell) => {
    const left = cell.startsWith(":");
    const right = cell.endsWith(":");
    if (left && right) return "center";
    if (right) return "right";
    if (left) return "left";
    return null;
  });
}

/** Turns heading text into a stable anchor, keeping CJK characters. */
function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[`*_~[\]()$]/g, "")
    .trim()
    .replace(/\s+/g, "-")
    .replace(/[^\p{L}\p{N}-]/gu, "");
}

function parseBlocks(source: string): Block[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  const seen = new Map<string, number>();
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();

    if (trimmed === "") {
      i++;
      continue;
    }

    if (trimmed.startsWith("```")) {
      const lang = trimmed.slice(3).trim();
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith("```")) {
        body.push(lines[i]);
        i++;
      }
      i++;
      blocks.push({ type: "code", lang, text: body.join("\n") });
      continue;
    }

    if (trimmed.startsWith("$$")) {
      if (trimmed.length > 4 && trimmed.endsWith("$$")) {
        blocks.push({ type: "math", tex: trimmed.slice(2, -2).trim() });
        i++;
        continue;
      }
      const body: string[] = [trimmed.slice(2)];
      i++;
      while (i < lines.length && !lines[i].trim().endsWith("$$")) {
        body.push(lines[i]);
        i++;
      }
      if (i < lines.length) body.push(lines[i].trim().slice(0, -2));
      i++;
      blocks.push({ type: "math", tex: body.join("\n").trim() });
      continue;
    }

    if (line.includes("|") && i + 1 < lines.length && TABLE_DIVIDER.test(lines[i + 1]) && lines[i + 1].includes("-")) {
      const header = splitRow(line);
      const align = parseAlign(lines[i + 1]);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].trim() !== "" && lines[i].includes("|")) {
        rows.push(splitRow(lines[i]));
        i++;
      }
      blocks.push({ type: "table", header, align, rows });
      continue;
    }

    const heading = /^(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (heading) {
      const text = heading[2];
      const base = slugify(text) || "section";
      const count = seen.get(base) ?? 0;
      seen.set(base, count + 1);
      blocks.push({ type: "heading", level: heading[1].length, text, id: count ? `${base}-${count}` : base });
      i++;
      continue;
    }

    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      blocks.push({ type: "rule" });
      i++;
      continue;
    }

    if (/^\s*>/.test(line)) {
      const body: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) {
        body.push(lines[i].replace(/^\s*>\s?/, ""));
        i++;
      }
      blocks.push({ type: "quote", text: body.join("\n") });
      continue;
    }

    const bullet = /^\s*[-*+]\s+(.*)$/;
    const ordered = /^\s*\d+[.)]\s+(.*)$/;
    if (bullet.test(line) || ordered.test(line)) {
      const isOrdered = ordered.test(line);
      const pattern = isOrdered ? ordered : bullet;
      const items: string[] = [];
      while (i < lines.length && pattern.test(lines[i])) {
        items.push(pattern.exec(lines[i])?.[1] ?? "");
        i++;
      }
      blocks.push({ type: "list", ordered: isOrdered, items });
      continue;
    }

    const body: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !/^(#{1,6}\s|```|\$\$|\s*>|\s*[-*+]\s|\s*\d+[.)]\s)/.test(lines[i])
    ) {
      body.push(lines[i]);
      i++;
    }
    if (body.length === 0) {
      body.push(lines[i]);
      i++;
    }
    blocks.push({ type: "paragraph", text: body.join("\n") });
  }

  return blocks;
}

/** Lists the headings in a document, in order, for a table of contents. */
export function markdownHeadings(source: string): Heading[] {
  return parseBlocks(source)
    .filter((block): block is Extract<Block, { type: "heading" }> => block.type === "heading")
    .map(({ id, level, text }) => ({ id, level, text }));
}

/** Renders TeX to HTML. KaTeX escapes its input and shows syntax errors in place, so this never throws. */
function renderTex(tex: string, displayMode: boolean): string {
  return katex.renderToString(tex, { displayMode, throwOnError: false, output: "htmlAndMathml" });
}

const SAFE_LINK = /^(https?:\/\/|mailto:)/i;

/** Renders inline Markdown as React nodes. Nothing is parsed as HTML. */
function inline(text: string, keyPrefix = "i"): ReactNode[] {
  const nodes: ReactNode[] = [];
  const pattern = /(`[^`]+`)|(\*\*[^*]+\*\*|__[^_]+__)|(\*[^*\n]+\*|_[^_\n]+_)|(\[[^\]]+\]\([^)\s]+\))|(\$(?!\s)[^$\n]+?(?<!\s)\$)/g;
  let last = 0;
  let match: RegExpExecArray | null;
  let n = 0;

  while ((match = pattern.exec(text)) !== null) {
    if (match.index > last) nodes.push(text.slice(last, match.index));
    const key = `${keyPrefix}-${n++}`;
    const token = match[0];
    if (match[1]) {
      nodes.push(<code key={key}>{token.slice(1, -1)}</code>);
    } else if (match[2]) {
      nodes.push(<strong key={key}>{inline(token.slice(2, -2), key)}</strong>);
    } else if (match[3]) {
      nodes.push(<em key={key}>{inline(token.slice(1, -1), key)}</em>);
    } else if (match[4]) {
      const link = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(token);
      if (link && SAFE_LINK.test(link[2])) {
        nodes.push(
          <a key={key} href={link[2]} target="_blank" rel="noreferrer noopener">
            {inline(link[1], key)}
          </a>,
        );
      } else {
        nodes.push(token);
      }
    } else {
      nodes.push(<span key={key} className="math-inline" dangerouslySetInnerHTML={{ __html: renderTex(token.slice(1, -1), false) }} />);
    }
    last = match.index + token.length;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

function withBreaks(text: string, keyPrefix: string): ReactNode[] {
  return text.split("\n").flatMap((line, index, all) => {
    const parts = inline(line, `${keyPrefix}-${index}`);
    return index < all.length - 1 ? [...parts, <br key={`${keyPrefix}-br-${index}`} />] : parts;
  });
}

/* ---------- Charts ---------- */

export interface ChartSpec {
  type: "bar" | "line" | "pie";
  title?: string;
  xTitle?: string;
  yTitle?: string;
  labels: string[];
  series: { name: string; data: number[] }[];
}

const CHART_COLORS = ["#d97757", "#3b82f6", "#6aa84f", "#8e7cc3", "#e69138", "#45818e"];

/** Validates a chart spec, returning an error message instead of throwing. */
export function parseChart(source: string): ChartSpec | string {
  let raw: unknown;
  try {
    raw = JSON.parse(source);
  } catch (error) {
    return `Invalid chart JSON: ${error instanceof Error ? error.message : "parse error"}`;
  }
  if (typeof raw !== "object" || raw === null) return "Chart spec must be an object.";
  const spec = raw as Partial<ChartSpec>;
  if (spec.type !== "bar" && spec.type !== "line" && spec.type !== "pie") return 'Chart type must be "bar", "line" or "pie".';
  if (!Array.isArray(spec.labels) || spec.labels.some((label) => typeof label !== "string")) return "labels must be an array of strings.";
  if (!Array.isArray(spec.series) || spec.series.length === 0) return "series must be a non-empty array.";
  for (const series of spec.series) {
    if (typeof series?.name !== "string" || !Array.isArray(series.data) || series.data.some((v) => typeof v !== "number" || !Number.isFinite(v))) {
      return "Each series needs a name and numeric data.";
    }
    if (series.data.length !== spec.labels.length) return "Each series must have one value per label.";
  }
  return {
    type: spec.type,
    title: typeof spec.title === "string" ? spec.title : undefined,
    xTitle: typeof spec.xTitle === "string" ? spec.xTitle : undefined,
    yTitle: typeof spec.yTitle === "string" ? spec.yTitle : undefined,
    labels: spec.labels,
    series: spec.series,
  };
}

function ChartView({ spec }: { spec: ChartSpec }) {
  const W = 640;
  const H = 320;
  const left = 48;
  const right = 16;
  const top = spec.title ? 36 : 16;
  const bottom = 40;
  const plotW = W - left - right;
  const plotH = H - top - bottom;
  const labelStyle = { fill: "var(--text-muted)", fontSize: 12 };
  const gridStyle = { stroke: "var(--border)" };

  let body: ReactNode;
  if (spec.type === "pie") {
    const values = spec.series[0].data.map((v) => Math.max(v, 0));
    const total = values.reduce((sum, v) => sum + v, 0);
    const cx = W / 2;
    const cy = top + plotH / 2;
    const r = Math.min(plotW, plotH) / 2;
    let angle = -Math.PI / 2;
    body = values.map((value, index) => {
      if (total === 0 || value === 0) return null;
      const span = (value / total) * Math.PI * 2;
      const color = CHART_COLORS[index % CHART_COLORS.length];
      if (span >= Math.PI * 2 - 1e-6) return <circle key={index} cx={cx} cy={cy} r={r} fill={color} />;
      const x0 = cx + r * Math.cos(angle);
      const y0 = cy + r * Math.sin(angle);
      angle += span;
      const x1 = cx + r * Math.cos(angle);
      const y1 = cy + r * Math.sin(angle);
      const large = span > Math.PI ? 1 : 0;
      const mid = angle - span / 2;
      const lx = cx + r * 0.6 * Math.cos(mid);
      const ly = cy + r * 0.6 * Math.sin(mid);
      return (
        <g key={index}>
          <path d={`M${cx} ${cy} L${x0} ${y0} A${r} ${r} 0 ${large} 1 ${x1} ${y1} Z`} fill={color} />
          <text x={lx} y={ly + 4} textAnchor="middle" style={{ fill: "#fff", fontSize: 12, fontWeight: 600 }}>
            {`${value} (${Math.round((value / total) * 100)}%)`}
          </text>
        </g>
      );
    });
  } else {
    const max = Math.max(...spec.series.flatMap((s) => s.data), 0) || 1;
    const yOf = (v: number) => top + plotH - (v / max) * plotH;
    const n = spec.labels.length;
    const groupW = plotW / Math.max(n, 1);
    const grid = [0, 1, 2, 3, 4].map((step) => {
      const value = (max * step) / 4;
      const y = yOf(value);
      return (
        <g key={step}>
          <line x1={left} x2={W - right} y1={y} y2={y} style={gridStyle} />
          <text x={left - 6} y={y + 4} textAnchor="end" style={labelStyle}>
            {Number.isInteger(value) ? value : value.toFixed(1)}
          </text>
        </g>
      );
    });
    const axisTitles = (
      <>
        {spec.xTitle && (
          <text x={left + plotW / 2} y={H - 6} textAnchor="middle" style={labelStyle}>
            {spec.xTitle}
          </text>
        )}
        {spec.yTitle && (
          <text x={14} y={top + plotH / 2} textAnchor="middle" transform={`rotate(-90 14 ${top + plotH / 2})`} style={labelStyle}>
            {spec.yTitle}
          </text>
        )}
      </>
    );
    const xLabels = spec.labels.map((label, index) => (
      <text key={index} x={left + groupW * (index + 0.5)} y={H - bottom + 18} textAnchor="middle" style={labelStyle}>
        {label}
      </text>
    ));

    if (spec.type === "bar") {
      const barW = (groupW * 0.7) / spec.series.length;
      body = (
        <>
          {grid}
          {spec.series.map((series, si) =>
            series.data.map((value, gi) => {
              const h = (value / max) * plotH;
              return (
                <g key={`${si}-${gi}`}>
                <rect
                  x={left + gi * groupW + groupW * 0.15 + si * barW}
                  y={top + plotH - h}
                  width={Math.max(barW - 2, 1)}
                  height={h}
                  rx={2}
                  fill={CHART_COLORS[si % CHART_COLORS.length]}
                />
                <text
                  x={left + gi * groupW + groupW * 0.15 + si * barW + Math.max(barW - 2, 1) / 2}
                  y={top + plotH - (value / max) * plotH - 4}
                  textAnchor="middle"
                  style={labelStyle}
                >
                  {value}
                </text>
                </g>
              );
            }),
          )}
          {xLabels}
          {axisTitles}
        </>
      );
    } else {
      body = (
        <>
          {grid}
          {spec.series.map((series, si) => {
            const color = CHART_COLORS[si % CHART_COLORS.length];
            const points = series.data.map((value, index) => ({
              x: n === 1 ? left + plotW / 2 : left + (index * plotW) / (n - 1),
              y: yOf(value),
            }));
            return (
              <g key={si}>
                <polyline fill="none" stroke={color} strokeWidth={2} points={points.map((p) => `${p.x},${p.y}`).join(" ")} />
                {points.map((p, index) => (
                  <g key={index}>
                    <circle cx={p.x} cy={p.y} r={3.5} fill={color} />
                    <text x={p.x} y={p.y - 8} textAnchor="middle" style={labelStyle}>
                      {series.data[index]}
                    </text>
                  </g>
                ))}
              </g>
            );
          })}
          {xLabels}
          {axisTitles}
        </>
      );
    }
  }

  const legend = spec.type === "pie" ? spec.labels : spec.series.map((s) => s.name);
  return (
    <figure className="chart">
      <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label={spec.title ?? `${spec.type} chart`}>
        {spec.title && (
          <text x={W / 2} y={22} textAnchor="middle" style={{ fill: "var(--text)", fontSize: 15, fontWeight: 600 }}>
            {spec.title}
          </text>
        )}
        {body}
      </svg>
      <figcaption className="chart-legend">
        {legend.map((name, index) => (
          <span key={`${name}-${index}`}>
            <i style={{ background: CHART_COLORS[index % CHART_COLORS.length] }} />
            {name}
          </span>
        ))}
      </figcaption>
    </figure>
  );
}

/* ---------- Document rendering ---------- */

/** Renders a Markdown document. Output is React elements only, so raw HTML stays as text. */
export function MarkdownView({ source }: { source: string }) {
  const blocks = parseBlocks(source);
  return (
    <article className="markdown">
      {blocks.map((block, index) => {
        const key = `b-${index}`;
        switch (block.type) {
          case "heading": {
            const Tag = `h${Math.min(block.level, 6)}` as "h1";
            return (
              <Tag key={key} id={block.id}>
                {inline(block.text, key)}
              </Tag>
            );
          }
          case "paragraph":
            return <p key={key}>{withBreaks(block.text, key)}</p>;
          case "list": {
            const items = block.items.map((item, itemIndex) => <li key={`${key}-${itemIndex}`}>{inline(item, `${key}-${itemIndex}`)}</li>);
            return block.ordered ? <ol key={key}>{items}</ol> : <ul key={key}>{items}</ul>;
          }
          case "code": {
            if (block.lang === "chart") {
              const parsed = parseChart(block.text);
              if (typeof parsed !== "string") return <ChartView key={key} spec={parsed} />;
              return (
                <div key={key} className="chart-error" role="alert">
                  {parsed}
                  <pre>
                    <code>{block.text}</code>
                  </pre>
                </div>
              );
            }
            return (
              <pre key={key}>
                <code>{block.text}</code>
              </pre>
            );
          }
          case "math":
            return <div key={key} className="math-block" dangerouslySetInnerHTML={{ __html: renderTex(block.tex, true) }} />;
          case "quote":
            return <blockquote key={key}>{withBreaks(block.text, key)}</blockquote>;
          case "table":
            return (
              <div key={key} className="table-wrap">
                <table>
                  <thead>
                    <tr>
                      {block.header.map((cell, ci) => (
                        <th key={ci} style={block.align[ci] ? { textAlign: block.align[ci] ?? undefined } : undefined}>
                          {inline(cell, `${key}-h${ci}`)}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {block.rows.map((row, ri) => (
                      <tr key={ri}>
                        {block.header.map((_, ci) => (
                          <td key={ci} style={block.align[ci] ? { textAlign: block.align[ci] ?? undefined } : undefined}>
                            {inline(row[ci] ?? "", `${key}-${ri}-${ci}`)}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            );
          case "rule":
            return <hr key={key} />;
        }
      })}
    </article>
  );
}

/** Table of contents with links to each heading. */
export function MarkdownToc({ headings }: { headings: Heading[] }) {
  if (headings.length === 0) return null;
  return (
    <nav className="markdown-toc" aria-label="Table of contents">
      <div className="markdown-toc-title">目录</div>
      <ol>
        {headings
          .filter((heading) => heading.level <= 2)
          .map((heading) => (
            <li key={heading.id}>
              <a
                href={`#${heading.id}`}
                onClick={(event) => {
                  event.preventDefault();
                  document.getElementById(heading.id)?.scrollIntoView({ behavior: "smooth", block: "start" });
                }}
              >
                {heading.text}
              </a>
            </li>
          ))}
      </ol>
    </nav>
  );
}
