// Notices codex sends about one chat that a component on screen needs, such as a change to the files of its
// checkout. Components subscribe for the chat they show; nothing has to be passed down through the screens.

type Listener = (method: string, params: Record<string, unknown>) => void;

const listeners = new Map<string, Set<Listener>>();

/** Calls `listener` for each notice the chat gets. Returns a function that stops the calls. */
export function subscribeThread(threadId: string, listener: Listener): () => void {
  const set = listeners.get(threadId) ?? new Set<Listener>();
  set.add(listener);
  listeners.set(threadId, set);
  return () => {
    set.delete(listener);
  };
}

/** Passes a notice to the components listening for its chat. */
export function publishThread(threadId: string, method: string, params: Record<string, unknown>): void {
  for (const listener of listeners.get(threadId) ?? []) listener(method, params);
}
