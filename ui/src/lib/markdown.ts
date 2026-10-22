// Markdown → sanitized HTML for assistant messages (marked + DOMPurify).

import DOMPurify from 'dompurify';
import { Marked, type Tokens } from 'marked';

const escape = (s: string) =>
  s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);

const md = new Marked({
  gfm: true,
  breaks: false,
  renderer: {
    code({ text, lang }: Tokens.Code) {
      const language = (lang ?? '').split(/\s/)[0];
      return `<div class="md-code"><div class="md-code-head"><span>${escape(language || 'text')}</span><button type="button" class="md-copy" data-copy>Copy</button></div><pre><code>${escape(text)}</code></pre></div>`;
    },
  },
});

DOMPurify.addHook('afterSanitizeAttributes', (node) => {
  if (node instanceof HTMLAnchorElement) {
    node.setAttribute('target', '_blank');
    node.setAttribute('rel', 'noopener noreferrer');
  }
});

const cache = new Map<string, string>();
const CACHE_MAX = 400;

/** Renders markdown; results for finished items are memoized by `cacheKey`. */
export function renderMarkdown(src: string, cacheKey?: string): string {
  if (cacheKey) {
    const hit = cache.get(cacheKey);
    if (hit !== undefined) return hit;
  }
  const html = DOMPurify.sanitize(md.parse(src, { async: false }) as string, { ADD_ATTR: ['target'] });
  if (cacheKey) {
    if (cache.size >= CACHE_MAX) cache.delete(cache.keys().next().value!);
    cache.set(cacheKey, html);
  }
  return html;
}
