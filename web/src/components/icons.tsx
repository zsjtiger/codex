import type { SVGProps } from "react";

// Small inline icon set (stroke icons, 20px grid), so no icon library is needed.
type IconProps = SVGProps<SVGSVGElement>;

const base = (props: IconProps) => ({
  width: 18,
  height: 18,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.8,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  "aria-hidden": true,
  ...props,
});

export const SidebarIcon = (p: IconProps) => (
  <svg {...base(p)}><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M9 4v16" /></svg>
);
export const PlusIcon = (p: IconProps) => (
  <svg {...base(p)}><path d="M12 5v14M5 12h14" /></svg>
);
export const SearchIcon = (p: IconProps) => (
  <svg {...base(p)}><circle cx="11" cy="11" r="6.5" /><path d="M16 16l4 4" /></svg>
);
export const ProjectIcon = (p: IconProps) => (
  <svg {...base(p)}><rect x="4" y="8" width="16" height="12" rx="2" /><path d="M7 8V6a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v2M4 12h16" /></svg>
);
export const ArtifactIcon = (p: IconProps) => (
  <svg {...base(p)}><rect x="7" y="3" width="13" height="13" rx="2" /><path d="M4 8v11a2 2 0 0 0 2 2h11" /></svg>
);
export const ClockIcon = (p: IconProps) => (
  <svg {...base(p)}><circle cx="12" cy="12" r="8.5" /><path d="M12 7.5V12l3 2" /></svg>
);
export const CustomizeIcon = (p: IconProps) => (
  <svg {...base(p)}><rect x="3.5" y="7" width="17" height="12" rx="2" /><path d="M9 7V5.5A1.5 1.5 0 0 1 10.5 4h3A1.5 1.5 0 0 1 15 5.5V7" /></svg>
);
export const MoreIcon = (p: IconProps) => (
  <svg {...base(p)}><circle cx="5" cy="12" r="1" /><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /></svg>
);
export const ChatIcon = (p: IconProps) => (
  <svg {...base(p)}><path d="M4 6.5A2.5 2.5 0 0 1 6.5 4h11A2.5 2.5 0 0 1 20 6.5v8a2.5 2.5 0 0 1-2.5 2.5H10l-4.5 3.5V17H6.5A2.5 2.5 0 0 1 4 14.5z" /></svg>
);
export const FileIcon = (p: IconProps) => (
  <svg {...base({ width: 14, height: 14, ...p })}><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5" /></svg>
);
export const PinIcon = (p: IconProps) => (
  <svg {...base({ width: 14, height: 14, ...p })}><path d="M9 4h6l-1 5 3 3v1H7v-1l3-3z" /><path d="M12 13v7" /></svg>
);
export const ChevronIcon = (p: IconProps) => (
  <svg {...base({ width: 14, height: 14, ...p })}><path d="M6 9l6 6 6-6" /></svg>
);
export const MicIcon = (p: IconProps) => (
  <svg {...base(p)}><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21" /></svg>
);
export const WaveIcon = (p: IconProps) => (
  <svg {...base(p)}><path d="M4 12v0M8 9v6M12 5v14M16 9v6M20 12v0" /></svg>
);
export const SparkIcon = (p: IconProps) => (
  <svg {...base({ width: 26, height: 26, ...p })} fill="currentColor" stroke="none"><path d="M12 2l1.6 6.4L20 10l-6.4 1.6L12 18l-1.6-6.4L4 10l6.4-1.6z" /></svg>
);
