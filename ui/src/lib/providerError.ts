// Turns omp's error texts (timeline error items, an agent's `error`) into something readable.
// A provider failure arrives as `400 {"type":"error","error":{"message":…},"request_id":…}`
// plus a `raw-http-request=<path>` line, and again wrapped in `Retries exhausted: <reason>.
// Original error: …` once omp gives up retrying. Both carry the same provider message, which
// becomes the text; everything else goes to the details.

import type { TimelineItem } from './protocol';

export interface ErrorDetail {
  label: string;
  value: string;
  /** Multi-line machine text (the response body). */
  code?: boolean;
}

export interface ProviderError {
  /** One readable sentence: the provider's message, or a summary of a limit error. */
  title: string;
  /** The provider's own message when `title` paraphrases it. */
  message?: string;
  /** A rate limit or usage quota: another model or a later retry helps. */
  limit: boolean;
  /** When a limit lifts, if the provider says so. */
  retryAt?: Date;
  details: ErrorDetail[];
  /** Equal for errors with the same cause (a failed request and its exhausted retries). */
  key: string;
}

const RETRIES = /^Retries exhausted:\s*/;
const ORIGINAL = /^([\s\S]*?)\s*Original error:\s*/;
const LOG = /^raw-http-request=(.*)$/gm;
const HTTP = /^(\d{3})\s+([[{][\s\S]*[\]}])$/;
const RATE = /rate.?limit|too many requests/i;
const QUOTA = /usage limit|quota|resource.?exhausted|credit balance|billing/i;
/** `2026-11-01 at 00:00 UTC`, `2026-11-01T00:00:00Z`, `2026-11-01 00:00 GMT`. */
const AT = /(\d{4}-\d{2}-\d{2})(?:T|\s+at\s+|\s+)(\d{1,2}):(\d{2})(?::(\d{2}))?(?:\.\d+)?\s*(?:UTC|GMT|Z)/i;
/** `Try again in ~158 min`, `try again in 20s`. */
const IN = /try again in\s*~?\s*(\d+(?:\.\d+)?)\s*(s|secs?|seconds?|m|mins?|minutes?|h|hrs?|hours?)\b/i;
const UNIT_MS: Record<string, number> = { s: 1000, m: 60_000, h: 3_600_000 };
/** Details in display order; the long response body last. */
const ORDER = ['Retry', 'HTTP status', 'Error type', 'Request ID', 'Request log', 'Response'];

type Json = Record<string, unknown>;
const obj = (v: unknown): Json | undefined => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Json) : undefined);
const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v.trim() : undefined);

function retryAt(text: string, at: number): Date | undefined {
  const abs = AT.exec(text);
  if (abs) {
    const d = new Date(`${abs[1]}T${abs[2].padStart(2, '0')}:${abs[3]}:${abs[4] ?? '00'}Z`);
    if (!Number.isNaN(d.getTime())) return d;
  }
  const rel = IN.exec(text);
  if (rel) return new Date(at + Number(rel[1]) * UNIT_MS[rel[2][0].toLowerCase()]);
  return undefined;
}

/** `at`: when the error happened (Unix ms), for relative times like "try again in 20 min". */
export function parseError(text: string, at = Date.now()): ProviderError {
  const details: ErrorDetail[] = [];
  let rest = text.trim();
  // The prompt's own error may carry the retry reason without the `Retries exhausted:` prefix.
  const retries = RETRIES.exec(rest);
  if (retries) rest = rest.slice(retries[0].length);
  const original = ORIGINAL.exec(rest);
  if (original) rest = rest.slice(original[0].length);
  if (original?.[1]) details.push({ label: 'Retry', value: `Gave up: ${original[1]}` });
  else if (retries) details.push({ label: 'Retry', value: 'All attempts failed' });
  for (const m of rest.matchAll(LOG)) details.push({ label: 'Request log', value: m[1].trim() });
  rest = rest.replace(LOG, '').trim();

  let message = rest;
  let status: string | undefined;
  let type: string | undefined;
  const http = HTTP.exec(rest);
  if (http) {
    status = http[1];
    details.push({ label: 'HTTP status', value: status });
    try {
      const parsed: unknown = JSON.parse(http[2]);
      const body = obj(Array.isArray(parsed) ? parsed[0] : parsed) ?? {};
      const err = obj(body.error);
      message = str(body.error) ?? str(err?.message) ?? str(body.message) ?? str(body.detail) ?? rest;
      type = str(err?.type) ?? str(err?.status) ?? str(err?.code) ?? (body.type !== 'error' ? str(body.type) : undefined);
      if (type) details.push({ label: 'Error type', value: type });
      const requestId = str(body.request_id) ?? str(body.requestId) ?? str(err?.request_id);
      if (requestId) details.push({ label: 'Request ID', value: requestId });
      details.push({ label: 'Response', value: JSON.stringify(parsed, null, 2), code: true });
    } catch {
      // Not JSON after all: the status line stays the message.
    }
  }
  if (!message) message = text.trim() || 'The request failed';

  const hay = `${message} ${type ?? ''}`;
  const limit = status === '429' || RATE.test(hay) || QUOTA.test(hay);
  const lifts = limit ? retryAt(message, at) : undefined;
  // A reset later today needs no date.
  const today = lifts?.toDateString() === new Date().toDateString();
  const reset = lifts?.toLocaleString(undefined, today ? { hour: '2-digit', minute: '2-digit' } : { day: 'numeric', month: 'numeric', year: 'numeric', hour: '2-digit', minute: '2-digit' });
  details.sort((a, b) => ORDER.indexOf(a.label) - ORDER.indexOf(b.label));
  return {
    title: limit ? `${QUOTA.test(hay) ? 'API limit reached' : 'Rate limit reached'}${reset ? ` — available again ${today ? 'at' : 'on'} ${reset}` : ''}` : message,
    message: limit ? message : undefined,
    limit,
    retryAt: lifts,
    details,
    key: message.replace(/\s+/g, ' '),
  };
}

/** One error from several with the same `key`: their details combined, without repeats. */
export function mergeErrors(errors: ProviderError[]): ProviderError {
  const [first, ...more] = errors;
  if (!more.length) return first;
  const seen = new Set<string>();
  const details = errors
    .flatMap((e) => e.details)
    .filter((d) => {
      const id = `${d.label}\u0000${d.value}`;
      return !seen.has(id) && !!seen.add(id);
    })
    .sort((a, b) => ORDER.indexOf(a.label) - ORDER.indexOf(b.label));
  return { ...first, details };
}

/** The latest turn (items after the last user message) already shows an error with this cause. */
export function errorInTimeline(text: string, items: TimelineItem[]): boolean {
  const key = parseError(text).key;
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === 'user') return false;
    if (it.kind === 'error' && it.text && parseError(it.text).key === key) return true;
  }
  return false;
}
