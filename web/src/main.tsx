import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { isDesktop, readStoredToken } from "./desktop";
import { loadWebToken, useTokenInMemory } from "./api/client";
import { LocaleProvider } from "./i18n";
import "./i18n-zh";
import "./styles.css";

// In the desktop app the saved session comes from the keychain before the first render.
async function start() {
  if (isDesktop()) {
    document.documentElement.classList.add("desktop");
    const stored = (await readStoredToken().catch(() => null)) ?? loadWebToken();
    useTokenInMemory(stored);
    if (stored) document.documentElement.dataset.session = "stored";
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <LocaleProvider>
        <App />
      </LocaleProvider>
    </StrictMode>,
  );
}
void start();
