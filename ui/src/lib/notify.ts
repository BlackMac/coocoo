// Notifications for agents that need input or finished while the page is hidden.
// Inside the desktop shell (`__COOCOO__.native`), the shell owns OS notifications.
//
// Web browsers only deliver notifications from a service worker; a suspended tab (mobile
// background) never fires `new Notification`. So the preferred path is Web Push: the
// daemon (this origin's) pushes to a subscription this page registers (`push.subscribe`),
// and `sw.js` shows it. Where Web Push is unavailable (plain HTTP LAN, no SW), the old
// in-tab `new Notification` remains as the fallback.
//
// Presence: `push.ack { visible }` tells the daemon this device is actively looking at
// Coocoo, so it skips pushing (the SW double-checks no window is visible).

import { fleet } from './fleet.svelte';
import { clientId, device } from './track';
import type { PushPrefs } from './protocol';

const PREF_KEY = 'coocoo.notify';
const PUSH_PREFS_KEY = 'coocoo.pushPrefs';

export function notificationsSupported(native: boolean): boolean {
  return !native && typeof Notification !== 'undefined';
}

/** iOS: only the installed Home Screen app can ask for notification permission. */
export function pushRequiresInstall(): boolean {
  if (typeof navigator === 'undefined' || !('serviceWorker' in navigator)) return false;
  const ios = /iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1);
  return ios && !window.matchMedia('(display-mode: standalone)').matches && !((navigator as Navigator & { standalone?: boolean }).standalone === true);
}

export function pushAvailable(): boolean {
  return (
    typeof navigator !== 'undefined' &&
    'serviceWorker' in navigator &&
    'PushManager' in window &&
    typeof Notification !== 'undefined' &&
    location.protocol === 'https:'
  );
}

export function notificationsEnabled(): boolean {
  try {
    return typeof Notification !== 'undefined' && Notification.permission === 'granted' && localStorage.getItem(PREF_KEY) !== 'off';
  } catch {
    return false;
  }
}

/** The device's saved notification preferences, sent with the subscription. */
export function pushPrefs(): PushPrefs {
  const on = { needsYou: true, finished: true, agentNames: true };
  try {
    const raw = localStorage.getItem(PUSH_PREFS_KEY);
    return raw ? { ...on, ...(JSON.parse(raw) as Partial<PushPrefs>) } : on;
  } catch {
    return on;
  }
}

export async function setPushPrefs(prefs: Partial<PushPrefs>): Promise<void> {
  const merged = { ...pushPrefs(), ...prefs };
  try {
    localStorage.setItem(PUSH_PREFS_KEY, JSON.stringify(merged));
  } catch {
    /* storage unavailable */
  }
  if (!swRegistration || !pushSubscription) return;
  await fleet.call('push.setPrefs', { clientId: clientId(), prefs: merged }).catch(() => {});
}

let swRegistration: ServiceWorkerRegistration | null = null;
let pushSubscription: PushSubscription | null = null;
let presenceTimer: number | undefined;

export async function setNotifications(on: boolean): Promise<boolean> {
  try {
    localStorage.setItem(PREF_KEY, on ? 'on' : 'off');
  } catch {
    /* storage unavailable */
  }
  if (!on) {
    await disablePush().catch(() => {});
    return false;
  }
  if (typeof Notification === 'undefined') return false;
  if (Notification.permission !== 'granted') {
    const answer = await Notification.requestPermission();
    if (answer !== 'granted') return false;
  }
  if (pushAvailable()) await enablePush().catch(() => {});
  return true;
}

async function enablePush(): Promise<void> {
  swRegistration = await navigator.serviceWorker.register('/sw.js');
  const state = await fleet.call('push.state', undefined);
  const key = urlB64ToUint8Array(state.vapidPublicKey);
  const existing = await swRegistration.pushManager.getSubscription();
  pushSubscription =
    existing ??
    (await swRegistration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: key.buffer as ArrayBuffer,
    }));
  await fleet.call('push.subscribe', {
    endpoint: pushSubscription.endpoint,
    keys: keysOf(pushSubscription),
    clientId: clientId(),
    device: device(),
    prefs: pushPrefs(),
  });
  startPresence();
}

async function disablePush(): Promise<void> {
  stopPresence();
  const sub = pushSubscription ?? (await swRegistration?.pushManager.getSubscription());
  if (sub) {
    await fleet.call('push.unsubscribe', { endpoint: sub.endpoint }).catch(() => {});
    await sub.unsubscribe().catch(() => {});
  }
  pushSubscription = null;
}

/** Reports visibility now and on every change; throttled to one call per 30 s per state. */
function startPresence(): void {
  stopPresence();
  const send = () => {
    const visible = document.visibilityState === 'visible';
    fleet.send('push.ack', { clientId: clientId(), visible });
  };
  send();
  document.addEventListener('visibilitychange', send);
  presenceTimer = window.setInterval(send, 30_000);
}

function stopPresence(): void {
  if (presenceTimer !== undefined) {
    clearInterval(presenceTimer);
    presenceTimer = undefined;
  }
}

function keysOf(sub: PushSubscription): { p256dh: string; auth: string } {
  const raw = sub.toJSON().keys as { p256dh?: string; auth?: string } | undefined;
  return { p256dh: raw?.p256dh ?? '', auth: raw?.auth ?? '' };
}

function urlB64ToUint8Array(base64: string): Uint8Array {
  const padding = '='.repeat((4 - (base64.length % 4)) % 4);
  const b64 = (base64 + padding).replace(/-/g, '+').replace(/_/g, '/');
  const raw = atob(b64);
  return Uint8Array.from(raw, (c) => c.charCodeAt(0));
}

// The service worker asks the (focused) page to open the agent a notification names.
if (typeof navigator !== 'undefined' && 'serviceWorker' in navigator && typeof window !== 'undefined') {
  navigator.serviceWorker.addEventListener('message', (event) => {
    if (event.data && event.data.coocoo === 'navigate' && typeof event.data.hash === 'string') {
      location.hash = event.data.hash;
    }
  });
}

/** Legacy fallback: shows a notification when the page is hidden; no service worker needed. */
export function notify(title: string, body: string, hash: string, tag: string): void {
  if (document.visibilityState === 'visible' || !notificationsEnabled()) return;
  if (pushAvailable() && swRegistration) return; // push already covers this device
  const n = new Notification(title, { body, tag, silent: false });
  n.onclick = () => {
    window.focus();
    location.hash = hash;
    n.close();
  };
}
