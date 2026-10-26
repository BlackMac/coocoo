// Files fetched from a host once and kept: refinement attachments (`boards.attachment`), the
// user's card context images (`boards.cardImage`) and the images of user messages and tool
// results (`agents.image`).

import { fleet } from './fleet.svelte';
import type { Attachment, TimelineImage } from './protocol';

/** Image data URLs and HTML sources by key (`''` = could not be loaded). */
export const files = $state<Record<string, string>>({});
const pending = new Set<string>();

/** Starts loading `k` unless it is loaded or loading, and returns it. */
function load(k: string, fetch: () => Promise<{ mime: string; data: string }>): string {
  if (!(k in files) && !pending.has(k)) {
    pending.add(k);
    fetch()
      .then(({ mime, data }) => {
        files[k] = mime === 'text/html' ? new TextDecoder().decode(Uint8Array.from(atob(data), (c) => c.charCodeAt(0))) : `data:${mime};base64,${data}`;
      })
      .catch(() => (files[k] = ''))
      .finally(() => pending.delete(k));
  }
  return k;
}

/**
 * Starts loading a card file and returns its key into `files`.
 * `version` tells files with the same name apart (a new refinement, a re-added image).
 */
export function loadCardFile(host: string, projectId: string, cardId: string, kind: 'attachment' | 'image', a: Attachment, version: string | number): string {
  return load(`${host}:${cardId}:${kind}:${version}:${a.name}`, () =>
    fleet.call(kind === 'image' ? 'boards.cardImage' : 'boards.attachment', { projectId, cardId, name: a.name }, host),
  );
}

/** Starts loading an image of an agent's timeline (user message, tool result) and returns its key into `files`. */
export function loadAgentImage(host: string, agentId: string, img: TimelineImage): string {
  return load(`${host}:agent:${agentId}:${img.name}`, () => fleet.call('agents.image', { id: agentId, name: img.name }, host));
}
