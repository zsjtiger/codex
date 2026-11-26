// Notices codex sends for an environment as a whole (its account's limits, its skills). Components that show
// those things subscribe here, so they refresh without the notices being passed down through every screen.

type Listener = (method: string, params: Record<string, unknown>) => void;

const listeners = new Map<string, Set<Listener>>();

/** Calls `listener` for each notice the environment gets. Returns a function that stops the calls. */
export function subscribeEnvironment(environmentId: string, listener: Listener): () => void {
  const set = listeners.get(environmentId) ?? new Set<Listener>();
  set.add(listener);
  listeners.set(environmentId, set);
  return () => {
    set.delete(listener);
  };
}

/** Passes a notice to the components listening for its environment. */
export function publishEnvironment(environmentId: string, method: string, params: Record<string, unknown>): void {
  for (const listener of listeners.get(environmentId) ?? []) listener(method, params);
}
