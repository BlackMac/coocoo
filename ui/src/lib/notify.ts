// Browser notifications for agents that need input or finished while the page is hidden.
// Inside the desktop shell (`__COOCOO__.native`), the shell owns OS notifications.

const PREF_KEY = 'coocoo.notify';

export function notificationsSupported(native: boolean): boolean {
  return !native && typeof Notification !== 'undefined';
}

export function notificationsEnabled(): boolean {
  try {
    return (
      typeof Notification !== 'undefined' && Notification.permission === 'granted' && localStorage.getItem(PREF_KEY) !== 'off'
    );
  } catch {
    return false;
  }
}

export async function setNotifications(on: boolean): Promise<boolean> {
  try {
    localStorage.setItem(PREF_KEY, on ? 'on' : 'off');
  } catch {
    /* storage unavailable */
  }
  if (!on || typeof Notification === 'undefined') return false;
  if (Notification.permission === 'default') await Notification.requestPermission();
  return Notification.permission === 'granted';
}

/** Shows a notification when the page is hidden; clicking it focuses the page at `hash`. */
export function notify(title: string, body: string, hash: string, tag: string): void {
  if (document.visibilityState === 'visible' || !notificationsEnabled()) return;
  const n = new Notification(title, { body, tag, silent: false });
  n.onclick = () => {
    window.focus();
    location.hash = hash;
    n.close();
  };
}
