// Placeholder data until the gateway API is wired in.

export interface ChatItem {
  id: string;
  title: string;
  /** Shows a document marker next to the title. */
  hasFile?: boolean;
  /** Shows a clock marker (scheduled). */
  scheduled?: boolean;
  /** Shows an unread dot. */
  unread?: boolean;
}

export interface Artifact {
  id: string;
  title: string;
  updated: string;
}

export const pinned: ChatItem[] = [
  { id: "p1", title: "Token-Filtered Light-TTPO 方法", hasFile: true },
  { id: "p2", title: "学位论文写作ing", hasFile: true },
  { id: "p3", title: "LaTeX Pandoc 工程模板", hasFile: true },
  { id: "p4", title: "模型微调与水印研究空白", hasFile: true },
  { id: "p5", title: "Step-aware masking for diffusion", hasFile: true },
  { id: "p6", title: "Academic paper writing for NeurIPS", hasFile: true },
  { id: "p7", title: "图Transformer的认证鲁棒性防御" },
  { id: "p8", title: "论文公式和推论的严格检查" },
  { id: "p9", title: "流形上的最速下降" },
  { id: "p10", title: "凸优化在模型微调中的应用" },
];

export const recents: ChatItem[] = [
  { id: "r1", title: "AlphaFold3与蛋白设计论文" },
  { id: "r2", title: "Flint 模型与 OPS5 比较" },
  { id: "r3", title: "Clash VLESS proxy config" },
  { id: "r4", title: "头晕恶心后走路恍惚" },
  { id: "r5", title: "Weekday Market Hotspots Report", scheduled: true },
  { id: "r6", title: "Codex开源情况" },
  { id: "r7", title: "Daily morning briefing", unread: true },
  { id: "r8", title: "全球与A股热点报告", hasFile: true },
  { id: "r9", title: "今日A股与全球资源热点", hasFile: true },
  { id: "r10", title: "开源大模型及训练数据" },
  { id: "r11", title: "Apertus 项目" },
  { id: "r12", title: "根据这个写教程" },
  { id: "r13", title: "恩施土司悬疑小说" },
  { id: "r14", title: "DuckDB分布式支持" },
  { id: "r15", title: "梦见抓住王锦蛇的预兆" },
  { id: "r16", title: "Author profile form setup" },
  { id: "r17", title: "Odoo maintenance and marketing_campaign" },
  { id: "r18", title: "决策智能开源系统" },
  { id: "r19", title: "SAPS-Trace GRPO 秩-2 坍塌" },
  { id: "r20", title: "Golang agent frameworks" },
];

export const artifacts: Artifact[] = [
  { id: "a1", title: "文本数据清洗操作手册", updated: "21h ago" },
  { id: "a2", title: "BIT 论文双格式工程使用说明", updated: "2d ago" },
  { id: "a3", title: "方向一：熵控制与 RLVR 安全漂移一研究方案", updated: "Oct 10" },
];

export const user = { name: "Tiger", initial: "T", plan: "Pro" };
