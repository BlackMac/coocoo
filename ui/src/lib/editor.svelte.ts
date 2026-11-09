// "Open in VS Code / Cursor" of an agent's or project's directory. A directory on the computer
// this client runs on opens as a local folder; one on another host over the editor's Remote-SSH
// (`ssh-remote+<target>`), where the target is what `ssh` on this computer accepts. Both the
// editor and the SSH targets belong to this device, so they are kept in localStorage.

import { fleet } from './fleet.svelte';
import type { HostInfo } from './protocol';
import { device } from './track';

export type EditorApp = 'vscode' | 'cursor' | 'off';

export const EDITORS: { id: EditorApp; label: string }[] = [
  { id: 'vscode', label: 'VS Code' },
  { id: 'cursor', label: 'Cursor' },
  { id: 'off', label: 'Off' },
];

const KEY = 'coocoo.editor';
const LOOPBACK = /^(127\.|localhost$|\[::1\]$|::1$)/;

interface Prefs {
  app: EditorApp;
  /** SSH target per host id, overriding `defaultSshTarget`. */
  ssh: Record<string, string>;
}

function load(): Prefs {
  try {
    const raw = localStorage.getItem(KEY);
    const p = raw ? (JSON.parse(raw) as Partial<Prefs>) : {};
    return { app: p.app ?? 'vscode', ssh: p.ssh ?? {} };
  } catch {
    return { app: 'vscode', ssh: {} };
  }
}

export const editor = $state<Prefs>(load());

function save() {
  try {
    localStorage.setItem(KEY, JSON.stringify($state.snapshot(editor)));
  } catch {
    /* storage unavailable */
  }
}

export function setEditorApp(app: EditorApp) {
  editor.app = app;
  save();
}

/** An empty target falls back to the default. */
export function setSshTarget(host: string, target: string) {
  const t = target.trim();
  if (t) editor.ssh[host] = t;
  else delete editor.ssh[host];
  save();
}

/** Whether `host` is the computer this client runs on, so its paths are local paths. */
export function isThisComputer(host: string): boolean {
  if (host !== fleet.homeId) return false;
  return fleet.native || LOOPBACK.test(location.hostname);
}

/** SSH target derived from how the fleet reaches the host, before any override. */
export function defaultSshTarget(h: HostInfo): string | undefined {
  if (h.kind === 'ssh') return h.address;
  if (h.kind === 'direct' && h.address) {
    const name = hostname(h.address);
    return name && !LOOPBACK.test(name) ? name : undefined;
  }
  // The home daemon, reached by this browser under some address.
  if (h.id === fleet.homeId && !fleet.native && !LOOPBACK.test(location.hostname)) return location.hostname;
  return undefined;
}

function hostname(address: string): string | undefined {
  try {
    return new URL(/^\w+:\/\//.test(address) ? address : `ws://${address}`).hostname || undefined;
  } catch {
    return undefined;
  }
}

export function sshTarget(host: string): string | undefined {
  const h = fleet.host(host);
  return editor.ssh[host] ?? (h ? defaultSshTarget(h) : undefined);
}

/** Menu label and URL that open `path` of `host` in the editor; `undefined` where it cannot. */
export function editorLink(host: string, path: string): { label: string; url: string } | undefined {
  const app = EDITORS.find((e) => e.id === editor.app);
  if (!app || app.id === 'off' || device() === 'phone') return undefined;
  const p = encodeURI(path);
  if (isThisComputer(host)) return { label: `Open in ${app.label}`, url: `${app.id}://file${p}` };
  const target = sshTarget(host);
  if (!target) return undefined;
  // `user@host` stays readable: VS Code takes the authority as written.
  const authority = encodeURIComponent(target).replace(/%40/g, '@');
  return { label: `Open in ${app.label} (SSH)`, url: `${app.id}://vscode-remote/ssh-remote+${authority}${p}` };
}

/** The desktop app hands new-window requests for editor schemes to the OS; a browser does the
 *  same for a plain navigation, without leaving the page. */
export function openEditor(url: string) {
  if (fleet.native) window.open(url, '_blank');
  else location.href = url;
}
