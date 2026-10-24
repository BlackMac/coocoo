// Copying text. The async clipboard API only exists in secure contexts (https, localhost); a
// phone reaching the daemon over plain http on the LAN has none, so it falls back to the
// selection-based `copy` command there.

export async function copyText(text: string): Promise<void> {
  if (window.isSecureContext && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      /* denied: try the fallback */
    }
  }
  const ta = document.createElement('textarea');
  ta.value = text;
  // Read-only: focusing it must not pop up the on-screen keyboard.
  ta.readOnly = true;
  ta.style.cssText = 'position:fixed;top:0;left:0;width:1px;height:1px;opacity:0;';
  const active = document.activeElement as HTMLElement | null;
  document.body.append(ta);
  ta.select();
  ta.setSelectionRange(0, text.length);
  const ok = document.execCommand('copy');
  ta.remove();
  active?.focus({ preventScroll: true });
  if (!ok) throw new Error('This browser does not allow copying here');
}
