// Placeholder data. Empty until the gateway API is connected.

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

export const pinned: ChatItem[] = [];
export const recents: ChatItem[] = [];
export const artifacts: Artifact[] = [];

/** The signed-in account, or null when no one is signed in. */
export const currentUser: { name: string; initial: string; plan: string } | null = null;
