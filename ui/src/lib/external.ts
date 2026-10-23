// Opening a URL outside Coocoo: a new browser tab on the web and on phones, the system browser
// in the desktop app (its shell turns every new-window request into an OS "open").

export function openExternal(url: string): void {
  window.open(url, '_blank', 'noopener,noreferrer');
}
