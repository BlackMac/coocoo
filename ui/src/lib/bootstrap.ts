// Resolves where the daemon lives and which token to use.
//
// Priority: `window.__COOCOO__` (injected by the Tauri shell) → `VITE_COOCOO_WS` (dev) →
// same-origin `/ws`. The token comes from the shell, `#token=…` (then persisted and stripped
// from the URL), localStorage, or `VITE_COOCOO_TOKEN` in dev; otherwise the UI asks for it.

/** Bootstrap object the native shell injects before the page loads. */
export interface NativeBootstrap {
  wsUrl: string;
  token: string;
  /** True inside the desktop shell: it owns OS notifications. */
  native?: boolean;
}

declare global {
  interface Window {
    __COOCOO__?: NativeBootstrap;
  }
}

export interface Endpoint {
  wsUrl: string;
  token: string | null;
  native: boolean;
}

const TOKEN_KEY = 'coocoo.token';

export function resolveEndpoint(): Endpoint {
  const injected = window.__COOCOO__;
  if (injected?.wsUrl) return { wsUrl: injected.wsUrl, token: injected.token, native: injected.native === true };

  const devUrl = import.meta.env.VITE_COOCOO_WS as string | undefined;
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const wsUrl = devUrl || `${proto}//${location.host}/ws`;

  const m = /(?:^#|[#&?])token=([^&]+)/.exec(location.hash);
  if (m) {
    const token = decodeURIComponent(m[1]);
    storeToken(token);
    history.replaceState(null, '', `${location.pathname}${location.search}#/`);
    return { wsUrl, token, native: false };
  }
  const stored = readStoredToken();
  const devToken = import.meta.env.VITE_COOCOO_TOKEN as string | undefined;
  return { wsUrl, token: stored ?? devToken ?? null, native: false };
}

export function storeToken(token: string): void {
  try {
    localStorage.setItem(TOKEN_KEY, token);
  } catch {
    /* storage unavailable (private mode): token lives for this page only */
  }
}

export function clearStoredToken(): void {
  try {
    localStorage.removeItem(TOKEN_KEY);
  } catch {
    /* ignore */
  }
}

function readStoredToken(): string | null {
  try {
    return localStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}
