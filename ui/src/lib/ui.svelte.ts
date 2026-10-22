// UI-only state: theme, toasts, dialogs, palette, drawer, composer drafts, panel layout.

import type { ImageInput, InspectResult } from './protocol';

export type ThemePref = 'system' | 'dark' | 'light';
export type PanelTab = 'changes' | 'browser' | 'plan' | 'info';

export interface Toast {
  id: number;
  kind: 'error' | 'info' | 'success';
  title: string;
  detail?: string;
}

/** Element captured in browser design mode, attached to the next prompt. */
export interface ElementChip {
  id: number;
  inspect: InspectResult;
  url: string;
}

export interface ImageChip {
  id: number;
  name: string;
  image: ImageInput;
}

export interface Draft {
  text: string;
  images: ImageChip[];
  elements: ElementChip[];
}

export type Dialog =
  | { kind: 'newAgent'; hostId?: string; projectId?: string }
  | { kind: 'addHost' }
  | { kind: 'addProject'; hostId?: string }
  | { kind: 'confirm'; title: string; message: string; action: string; danger?: boolean; run: () => void | Promise<void> };

const THEME_KEY = 'coocoo.theme';
const PANEL_KEY = 'coocoo.panel';

function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function save(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* storage unavailable */
  }
}

let seq = 1;

class Ui {
  themePref = $state<ThemePref>(load<ThemePref>(THEME_KEY, 'system'));
  systemDark = $state(matchMedia('(prefers-color-scheme: dark)').matches);
  theme = $derived<'dark' | 'light'>(this.themePref === 'system' ? (this.systemDark ? 'dark' : 'light') : this.themePref);

  toasts = $state<Toast[]>([]);
  dialog = $state<Dialog | null>(null);
  paletteOpen = $state(false);
  drawerOpen = $state(false);
  panel = $state<{ tab: PanelTab; width: number; open: boolean }>(
    load(PANEL_KEY, { tab: 'changes' as PanelTab, width: 460, open: true }),
  );
  /** Mobile agent view: which pane fills the screen. */
  mobilePane = $state<'chat' | PanelTab>('chat');
  drafts = $state<Record<string, Draft>>({});

  constructor() {
    matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (e) => {
      this.systemDark = e.matches;
    });
    $effect.root(() => {
      $effect(() => {
        document.documentElement.dataset.theme = this.theme;
        document.querySelector('meta[name="theme-color"]')?.setAttribute('content', this.theme === 'dark' ? '#0b0c0e' : '#ffffff');
      });
      $effect(() => save(PANEL_KEY, $state.snapshot(this.panel)));
    });
  }

  setTheme(pref: ThemePref) {
    this.themePref = pref;
    save(THEME_KEY, pref);
  }

  toggleTheme() {
    this.setTheme(this.theme === 'dark' ? 'light' : 'dark');
  }

  toast(kind: Toast['kind'], title: string, detail?: string) {
    const id = seq++;
    this.toasts.push({ id, kind, title, detail });
    if (this.toasts.length > 4) this.toasts.shift();
    setTimeout(() => this.dismiss(id), kind === 'error' ? 7000 : 3500);
  }

  /** Reports a failed action; `err` is usually an `RpcError`. */
  error(title: string, err: unknown) {
    const detail = err instanceof Error ? err.message : typeof err === 'string' ? err : undefined;
    this.toast('error', title, detail);
  }

  dismiss(id: number) {
    const i = this.toasts.findIndex((t) => t.id === id);
    if (i >= 0) this.toasts.splice(i, 1);
  }

  draft(key: string): Draft {
    // Read back through the store so callers get the reactive proxy, not the raw literal.
    if (!this.drafts[key]) this.drafts[key] = { text: '', images: [], elements: [] };
    return this.drafts[key];
  }

  attachElement(key: string, inspect: InspectResult, url: string) {
    this.draft(key).elements.push({ id: seq++, inspect, url });
  }

  attachImage(key: string, name: string, image: ImageInput) {
    this.draft(key).images.push({ id: seq++, name, image });
  }

  showPanel(tab: PanelTab) {
    this.panel.tab = tab;
    this.panel.open = true;
    this.mobilePane = tab;
  }
}

export const ui = new Ui();
