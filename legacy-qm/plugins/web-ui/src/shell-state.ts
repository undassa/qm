export type AuthMode = "portal" | "dev";

export interface Me {
  user: string;
  org: string;
  mode?: AuthMode;
  slackWorkspaceUrl?: string | null;
  impersonatedBy?: string | null;
  permissions?: string[];
  individualModelAuth?: boolean;
  modelAuthConnected?: boolean;
}

const VIEWS = [
  "chats",
  "contexts",
  "webhooks",
  "crons",
  "files",
  "keychain",
  "deploys",
  "memory",
  "skills",
  "documents",
] as const;
export type View = (typeof VIEWS)[number];

export function isView(view: string | null | undefined): view is View {
  return (VIEWS as readonly (string | null | undefined)[]).includes(view);
}

const ACTIVE_SCOPE_KEY = "mh.activeScope";

function storedScope(): string | null {
  try {
    return localStorage.getItem(ACTIVE_SCOPE_KEY);
  } catch {
    return null;
  }
}

export const appState = {
  me: null as Me | null,
  activeScope: storedScope(),
  currentView: "chats" as View,
  viewRenderSeq: 0,
  topEl: null as HTMLElement | null,
  listEl: null as HTMLElement | null,
  mainEl: null as HTMLElement | null,
};

export function setActiveScope(scopeId: string | null): void {
  appState.activeScope = scopeId;
  try {
    if (scopeId) localStorage.setItem(ACTIVE_SCOPE_KEY, scopeId);
    else localStorage.removeItem(ACTIVE_SCOPE_KEY);
  } catch {
    void 0;
  }
}

export function scopeOfProjectId(projectId: string): string {
  return `group:web-project-${projectId}`;
}

export function projectIdOfScope(scopeId: string | null): string | null {
  const prefix = "group:web-project-";
  return scopeId && scopeId.startsWith(prefix) ? scopeId.slice(prefix.length) : null;
}

export function can(key: string): boolean {
  return appState.me?.permissions?.includes(key) === true;
}
