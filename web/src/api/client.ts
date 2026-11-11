import { tx } from "../i18n";
import { isDesktop, writeStoredToken } from "../desktop";
import { apiUrl } from "./config";
// Thin fetch wrapper for the gateway. The session token lives in localStorage so a
// reload keeps the user signed in. This is per-browser convenience, not a secret store.

const SESSION_KEY = "codex.session";

let token: string | null = null;

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    // Gateway messages that have a translation are shown in the app's language.
    super(tx(message));
  }
}

/** The token kept in web storage. Used in browsers, and in the desktop app when no keychain is available. */
export function loadWebToken(): string | null {
  try {
    return localStorage.getItem(SESSION_KEY);
  } catch {
    return null;
  }
}

export function loadStoredToken(): string | null {
  // In the desktop app the token was read from the keychain at start-up (see main.tsx).
  if (isDesktop()) return token;
  return loadWebToken();
}

/** The desktop app keeps the token in the keychain; this sets it for the page's requests. */
export function useTokenInMemory(next: string | null): void {
  token = next;
}

export function getToken(): string | null {
  return token;
}

export function setToken(next: string | null): void {
  token = next;
  // The desktop app keeps the token in the keychain; web storage is the fallback when it is unavailable.
  if (isDesktop()) {
    writeStoredToken(next).catch(() => {
      try {
        if (next) localStorage.setItem(SESSION_KEY, next);
        else localStorage.removeItem(SESSION_KEY);
      } catch {
        // No storage at all: the token lasts for this window only.
      }
    });
    return;
  }
  try {
    if (next) localStorage.setItem(SESSION_KEY, next);
    else localStorage.removeItem(SESSION_KEY);
  } catch {
    // Storage can be unavailable (private mode); the token then lasts for this page only.
  }
}

export async function api<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {};
  if (body !== undefined) headers["content-type"] = "application/json";
  if (token) headers.authorization = `Bearer ${token}`;

  const response = await fetch(apiUrl(path), {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status === 204) return undefined as T;

  const data = await response.json().catch(() => null);
  if (!response.ok) {
    throw new ApiError(
      response.status,
      data?.error?.code ?? "error",
      data?.error?.message ?? response.statusText,
    );
  }
  return data as T;
}
