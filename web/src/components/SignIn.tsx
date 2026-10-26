import { useState } from "react";
import { ApiError } from "../api/client";
import { SparkIcon } from "./icons";

export function SignIn({ onSignIn }: { onSignIn: (email: string) => Promise<void> }) {
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  return (
    <main className="signin">
      <form
        className="signin-card"
        onSubmit={async (event) => {
          event.preventDefault();
          setBusy(true);
          setError(null);
          try {
            await onSignIn(email.trim());
          } catch (err) {
            setError(
              err instanceof ApiError && err.status === 404
                ? "Development sign-in is disabled on this gateway. Start it with GATEWAY_DEV_LOGIN=1."
                : err instanceof Error
                  ? err.message
                  : "Sign-in failed.",
            );
          } finally {
            setBusy(false);
          }
        }}
      >
        <SparkIcon className="signin-mark" />
        <h1>Sign in</h1>
        <label className="signin-field">
          <span>Email</span>
          <input
            type="email"
            required
            autoComplete="email"
            value={email}
            onChange={(event) => setEmail(event.target.value)}
            placeholder="you@example.com"
          />
        </label>
        <button className="primary" type="submit" disabled={busy || !email.trim()}>
          {busy ? "Signing in…" : "Continue"}
        </button>
        {error && <p className="banner error">{error}</p>}
        <p className="signin-note">Development sign-in: the gateway does not check a password.</p>
      </form>
    </main>
  );
}
