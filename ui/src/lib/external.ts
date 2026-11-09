// Opening things outside Coocoo. URLs: a new browser tab on the web and on phones, the system
// browser in the desktop app (its shell turns every new-window request into an OS "open").
// Directories: the file manager, only in the desktop app.

import { invoke } from '@tauri-apps/api/core';
import { fleet } from './fleet.svelte';
import { ui } from './ui.svelte';

export function openExternal(url: string): void {
  window.open(url, '_blank', 'noopener,noreferrer');
}

/**
 * Label of the desktop app's "open this directory in the file manager" entry, or `undefined`
 * where there is none: in the web UI, and for hosts other than the computer the app runs on.
 */
export function folderLabel(host: string): string | undefined {
  if (!fleet.native || host !== fleet.homeId) return undefined;
  const ua = navigator.userAgent;
  return /Mac/.test(ua) ? 'Open in Finder' : /Windows/.test(ua) ? 'Open in Explorer' : 'Open in file manager';
}

/** Opens a directory of the local host in the file manager (desktop app only). */
export function openFolder(path: string): void {
  invoke('open_folder', { path }).catch((err) => ui.error('Cannot open the folder', err));
}
