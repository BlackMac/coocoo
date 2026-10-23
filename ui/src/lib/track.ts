// Client side of the usage log (`insights.track`): which views, panels, dialogs and commands are
// used, from which kind of device. Never content: only ids such as a panel or command name.
//
// Events are buffered and sent to the home daemon every 10 s and when the page is hidden; the
// daemon drops them while the log is switched off.

import { fleet } from './fleet.svelte';
import type { Device, TrackEvent } from './protocol';

const FLUSH_MS = 10_000;
const MAX_BUFFER = 500;
const CLIENT_KEY = 'coocoo.clientId';

let buffer: TrackEvent[] = [];
let timer: number | undefined;
let started = false;

/** Random id of this client installation (not of the user). */
export function clientId(): string {
  let id = localStorage.getItem(CLIENT_KEY);
  if (!id) {
    id = Array.from(crypto.getRandomValues(new Uint8Array(6)), (b) => b.toString(16).padStart(2, '0')).join('');
    localStorage.setItem(CLIENT_KEY, id);
  }
  return id;
}

/** `desktop` = the Coocoo app, `phone` = a touch device with a narrow screen, else `browser`. */
export function device(): Device {
  if (fleet.native) return 'desktop';
  const touch = matchMedia('(pointer: coarse)').matches;
  return touch && Math.min(screen.width, screen.height) < 760 ? 'phone' : 'browser';
}

export interface TrackRef {
  agentId?: string;
  projectId?: string;
  cardId?: string;
  meta?: Record<string, string | number | boolean>;
}

/** Record a client event (`kind` without the `ui.` prefix, e.g. `panel`). */
export function track(kind: string, ref: TrackRef = {}): void {
  start();
  if (buffer.length >= MAX_BUFFER) buffer.shift();
  buffer.push({ ts: Date.now(), kind: `ui.${kind}`, ...ref });
  timer ??= window.setTimeout(flush, FLUSH_MS);
}

function flush(): void {
  clearTimeout(timer);
  timer = undefined;
  if (!buffer.length || !fleet.client || fleet.conn !== 'open') return;
  const events = buffer;
  buffer = [];
  fleet.call('insights.track', { clientId: clientId(), device: device(), events }).catch(() => {
    // Keep them for the next attempt (bounded).
    buffer = [...events, ...buffer].slice(-MAX_BUFFER);
  });
}

function start(): void {
  if (started) return;
  started = true;
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') {
      buffer.push({ ts: Date.now(), kind: 'ui.leave' });
      flush();
    } else {
      track('return');
    }
  });
}
