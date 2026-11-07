import { useState } from "react";
import { ApiError } from "../api/client";
import { SparkIcon } from "./icons";

interface SignInProps {
  onSignIn: (username: string, password: string) => Promise<void>;
  onRegister: (details: { username: string; email: string; password: string; display_name: string }) => Promise<void>;
}

/** Sign in with a username and password, or create an account. */
export function SignIn({ onSignIn, onRegister }: SignInProps) {
  const [mode, setMode] = useState<"signin" | "register">("signin");
  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const registering = mode === "register";

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      if (registering) {
        await onRegister({ username: username.trim(), email: email.trim(), password, display_name: displayName.trim() || username.trim() });
      } else {
        await onSignIn(username.trim(), password);
      }
    } catch (err) {
      setError(err instanceof ApiError ? err.message : err instanceof Error ? err.message : "Sign-in failed.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <main className="signin">
      <form
        className="signin-card"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <SparkIcon className="signin-mark" />
        <h1>{registering ? "Create account" : "Sign in"}</h1>
        <label className="signin-field">
          <span>Username</span>
          <input
            required
            autoComplete="username"
            value={username}
            onChange={(event) => setUsername(event.target.value)}
            placeholder="your-name"
            pattern="[A-Za-z0-9_.\-]{3,32}"
            title="3-32 letters, digits, _, . or -"
          />
        </label>
        {registering && (
          <>
            <label className="signin-field">
              <span>Email</span>
              <input type="email" required autoComplete="email" value={email} onChange={(event) => setEmail(event.target.value)} placeholder="you@example.com" />
            </label>
            <label className="signin-field">
              <span>Display name</span>
              <input value={displayName} onChange={(event) => setDisplayName(event.target.value)} placeholder="Optional" />
            </label>
          </>
        )}
        <label className="signin-field">
          <span>Password</span>
          <input
            type="password"
            required
            minLength={8}
            autoComplete={registering ? "new-password" : "current-password"}
            value={password}
            onChange={(event) => setPassword(event.target.value)}
          />
        </label>
        <button className="primary" type="submit" disabled={busy || !username.trim() || !password}>
          {busy ? "Please wait…" : registering ? "Create account" : "Sign in"}
        </button>
        {error && <p className="banner error">{error}</p>}
        <p className="signin-note">
          {registering ? "Already have an account? " : "No account yet? "}
          <button type="button" className="link-button" onClick={() => { setMode(registering ? "signin" : "register"); setError(null); }}>
            {registering ? "Sign in" : "Create one"}
          </button>
        </p>
      </form>
    </main>
  );
}
