import { useState } from "react";
import { setGatewayBase } from "../api/config";

/** Asks the desktop app which gateway to use, checks it answers, then reloads into the app. */
export function GatewaySetup() {
  const [address, setAddress] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const connect = async () => {
    const value = address.trim().replace(/\/+$/, "");
    let url: URL;
    try {
      url = new URL(value);
    } catch {
      setError("Enter the full address, for example https://codex.example.com");
      return;
    }
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      setError("The address must start with http:// or https://");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const response = await fetch(`${value}/healthz`);
      if (!response.ok) throw new Error(`The gateway answered with ${response.status}.`);
    } catch (err) {
      setBusy(false);
      setError(err instanceof Error && err.message.startsWith("The gateway") ? err.message : "Could not reach that gateway. Check the address and that it is running.");
      return;
    }
    setGatewayBase(value);
    window.location.reload();
  };

  return (
    <main className="gateway-setup">
      <form
        className="form-card gateway-setup-card"
        onSubmit={(event) => {
          event.preventDefault();
          void connect();
        }}
      >
        <h1>Connect to Baize</h1>
        <p className="muted small">Enter the address of the gateway this app should use.</p>
        <label className="field">
          <span>Gateway address</span>
          <input value={address} onChange={(e) => setAddress(e.target.value)} placeholder="https://codex.example.com" autoFocus required />
        </label>
        {error && <p className="banner error small" role="alert">{error}</p>}
        <div className="form-actions">
          <button className="primary" type="submit" disabled={busy || !address.trim()}>
            {busy ? "Checking…" : "Connect"}
          </button>
        </div>
      </form>
    </main>
  );
}

/** True when running inside the Tauri desktop shell. */
export function isDesktopApp(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
