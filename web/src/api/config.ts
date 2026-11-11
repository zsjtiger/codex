// Where the gateway is. The web app is served by the gateway, so by default it uses the page's
// own origin. The desktop app (and any other front end on another origin) sets the gateway
// address, either in the build (VITE_GATEWAY_URL) or in its settings (localStorage).

const GATEWAY_KEY = "codex.gatewayUrl";

/** The gateway's base address with no trailing slash. Empty means the page's own origin. */
export function gatewayBase(): string {
  let saved: string | null = null;
  try {
    saved = localStorage.getItem(GATEWAY_KEY);
  } catch {
    // Storage can be unavailable; fall back to the build setting.
  }
  const configured = saved || (import.meta.env.VITE_GATEWAY_URL as string | undefined) || "";
  return configured.replace(/\/+$/, "");
}

/** Sets the gateway address for this app. An empty value goes back to the page's origin. */
export function setGatewayBase(value: string): void {
  try {
    if (value.trim()) localStorage.setItem(GATEWAY_KEY, value.trim().replace(/\/+$/, ""));
    else localStorage.removeItem(GATEWAY_KEY);
  } catch {
    // Storage can be unavailable; the setting then lasts for this session only.
  }
}

/** The full URL of a REST route, such as apiUrl("/me"). */
export function apiUrl(path: string): string {
  return `${gatewayBase()}/api/v1${path}`;
}

/** The WebSocket URL for a ticket. Uses ws/wss to match the gateway address. */
export function wsUrl(ticket: string): string {
  const base = gatewayBase();
  const query = `/ws?ticket=${encodeURIComponent(ticket)}`;
  if (base) return `${base.replace(/^http/, "ws")}${query}`;
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  return `${scheme}://${location.host}${query}`;
}
