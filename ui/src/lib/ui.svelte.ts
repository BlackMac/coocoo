// UI-only state: theme, toasts, dialogs, palette, drawer, composer drafts, panel layout.

import type { ImageInput, InspectResult } from './protocol';
import { track, type TrackRef } from './track';

export type ThemePref = 'system' | 'dark' | 'light';
export type PanelTab = 'changes' | 'browser' | 'terminal' | 'board' | 'plan' | 'info';

export interface Toast {
  id: number;
  kind: 'error' | 'info' | 'success';
  title: string;
  detail?: string;
  /** A button in the toast; clicking it runs `run` and dismisses the toast. */
  action?: ToastAction;
}

export interface ToastAction {
  label: string;
  run: () => unknown;
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
  | { kind: 'subagent'; hostId: string; agentId: string; subagentId: string; sessionFile: string }
  | { kind: 'confirm'; title: string; message: string; action: string; danger?: boolean; run: () => void | Promise<void> }
  /** Push the project checkout an agent works in, after listing the outgoing commits. */
  | { kind: 'push'; hostId: string; agentId: string }
  /** Create (without `scheduleId`) or edit a project's scheduled run. */
  | { kind: 'schedule'; hostId: string; projectId: string; scheduleId?: string }
  /** Create (without `name`) or replace a project's secret. */
  | { kind: 'secret'; hostId: string; projectId: string; name?: string };

const THEME_KEY = 'coocoo.theme';
const PANEL_KEY = 'coocoo.panel';
const TERM_FONT_KEY = 'coocoo.termFontSize';

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
  #dialog = $state<Dialog | null>(null);
  /** Set by `submitDialog` so the closing setter records a submit instead of a cancel. */
  #submitting = false;
  #paletteOpen = $state(false);
  drawerOpen = $state(false);
  panel = $state<{ tab: PanelTab; width: number; open: boolean }>(
    load(PANEL_KEY, { tab: 'changes' as PanelTab, width: 460, open: true }),
  );
  /** Mobile agent view: which pane fills the screen. */
  mobilePane = $state<'chat' | PanelTab>('chat');
  drafts = $state<Record<string, Draft>>({});
  /** A terminal the next TerminalPanel for its directory should show (set when an action starts). */
  terminalFocus = $state<{ host: string; id: string } | null>(null);
  /** A browser tab the BrowserPanel should select (set when a URL action opened it). */
  browserFocus = $state<{ host: string; profile: string; targetId: string } | null>(null);
  /** An agent (key) whose composer should open its model picker (an error block asked for another model). */
  modelPicker = $state<string | null>(null);
  /** Terminal font size in px (pinch on a touch screen changes it). */
  termFontSize = $state(load(TERM_FONT_KEY, 12.5));

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
      $effect(() => save(TERM_FONT_KEY, this.termFontSize));
    });
  }

  // Every open and close of a dialog or the palette goes through these setters, so the usage log
  // sees them without each call site having to remember.
  get dialog(): Dialog | null {
    return this.#dialog;
  }

  set dialog(next: Dialog | null) {
    const prev = this.#dialog;
    if (prev && prev !== next) track(this.#submitting ? 'dialog.submit' : 'dialog.cancel', { meta: { dialog: prev.kind } });
    if (next && prev !== next) track('dialog.open', { meta: { dialog: next.kind } });
    this.#submitting = false;
    this.#dialog = next;
  }

  /** Closes the dialog after it did what it is for (as opposed to being dismissed). */
  submitDialog() {
    this.#submitting = true;
    this.dialog = null;
  }

  get paletteOpen(): boolean {
    return this.#paletteOpen;
  }

  set paletteOpen(open: boolean) {
    if (open && !this.#paletteOpen) track('palette.open');
    this.#paletteOpen = open;
  }

  setTheme(pref: ThemePref) {
    this.themePref = pref;
    save(THEME_KEY, pref);
  }

  toggleTheme() {
    this.setTheme(this.theme === 'dark' ? 'light' : 'dark');
  }

  toast(kind: Toast['kind'], title: string, detail?: string, action?: ToastAction) {
    const id = seq++;
    this.toasts.push({ id, kind, title, detail, action });
    if (this.toasts.length > 4) this.toasts.shift();
    // With a button there is something to do: leave time to do it.
    setTimeout(() => this.dismiss(id), action ? 12000 : kind === 'error' ? 7000 : 3500);
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

  /** Puts a prompt that was not sent back into the composer, after whatever is drafted already. */
  restoreDraft(key: string, from: Draft) {
    const d = this.draft(key);
    d.text = [d.text.trimEnd(), from.text].filter(Boolean).join('\n\n');
    for (const im of from.images) d.images.push({ ...im, id: seq++ });
    for (const el of from.elements) d.elements.push({ ...el, id: seq++ });
  }

  /** Opens a side panel; `by` (the agent/project it is opened in) marks a user's choice for the usage log. */
  showPanel(tab: PanelTab, by?: Pick<TrackRef, 'agentId' | 'projectId'>) {
    if (by) track('panel', { ...by, meta: { panel: tab } });
    this.panel.tab = tab;
    this.panel.open = true;
    this.mobilePane = tab;
  }
}

export const ui = new Ui();
