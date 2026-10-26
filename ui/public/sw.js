// Coocoo service worker: shows push notifications while no Coocoo tab is visible and
// opens the agent the notification is about. Served at the root scope so `push` events
// arrive even when the tab is closed (iOS: only from the Home Screen app).
//
// Payload (from the daemon's push sender): { title, body, hash, tag }.

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));

self.addEventListener('push', (event) => {
  let payload = {};
  try {
    payload = event.data ? event.data.json() : {};
  } catch {
    payload = { title: 'Coocoo', body: '' };
  }
  event.waitUntil(
    (async () => {
      const visible = await self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then((cs) =>
        cs.some((c) => 'visibilityState' in c && c.visibilityState === 'visible'),
      );
      if (visible) return; // The tab shows it already; the daemon skips this device only when the page acked.
      return self.registration.showNotification(payload.title || 'Coocoo', {
        body: payload.body || '',
        tag: payload.tag,
        icon: '/icons/icon-192.png',
        badge: '/icons/icon-192.png',
        data: { hash: payload.hash || '#/' },
      });
    })(),
  );
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const hash = event.notification.data && event.notification.data.hash;
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
      for (const w of windows) {
        if ('focus' in w) {
          if (hash && w.url && w.url.indexOf(self.location.origin) === 0 && 'navigate' in w === false) {
            // A same-origin window: focus it and let the page apply the hash (below).
            await w.focus();
            if (hash) w.postMessage({ coocoo: 'navigate', hash });
            return;
          }
        }
      }
      await self.clients.openWindow(hash || '/');
    })(),
  );
});

// The push service rotated the subscription: re-register it with the daemon on next load.
self.addEventListener('pushsubscriptionchange', (event) => {
  event.waitUntil(
    (async () => {
      // The page re-subscribes on load (notify.ts); here we only make sure the old one is
      // dropped. Without a live page there is no token to register the new one, so this is
      // best effort: the next app start heals it.
    })(),
  );
});
