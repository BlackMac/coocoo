// Files stored with board cards, loaded once and kept: refinement attachments (`boards.attachment`)
// and the user's context images (`boards.cardImage`).

import { fleet } from './fleet.svelte';
import type { Attachment } from './protocol';

/** Image data URLs and HTML sources by key (`''` = could not be loaded). */
export const cardFiles = $state<Record<string, string>>({});
const pending = new Set<string>();

/**
 * Starts loading `a` unless it is loaded or loading, and returns its key into `cardFiles`.
 * `version` tells files with the same name apart (a new refinement, a re-added image).
 */
export function loadCardFile(host: string, projectId: string, cardId: string, kind: 'attachment' | 'image', a: Attachment, version: string | number): string {
  const k = `${host}:${cardId}:${kind}:${version}:${a.name}`;
  if (!(k in cardFiles) && !pending.has(k)) {
    pending.add(k);
    fleet
      .call(kind === 'image' ? 'boards.cardImage' : 'boards.attachment', { projectId, cardId, name: a.name }, host)
      .then(({ mime, data }) => {
        cardFiles[k] = mime === 'text/html' ? new TextDecoder().decode(Uint8Array.from(atob(data), (c) => c.charCodeAt(0))) : `data:${mime};base64,${data}`;
      })
      .catch(() => (cardFiles[k] = ''))
      .finally(() => pending.delete(k));
  }
  return k;
}
