/// <reference types="svelte" />
/// <reference types="vite/client" />

/** UI package version, injected by Vite. */
declare const __APP_VERSION__: string;

interface ImportMetaEnv {
  /** Dev only: daemon WebSocket URL (e.g. ws://127.0.0.1:7421/ws). */
  readonly VITE_COOCOO_WS?: string;
  /** Dev only: access token used when none is stored. */
  readonly VITE_COOCOO_TOKEN?: string;
}
