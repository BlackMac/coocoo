// WebSocket client for the Coocoo daemon protocol (docs/PROTOCOL.md).
//
// Request/response correlation by numeric id with per-request timeouts, event fan-out to
// subscribers, and automatic reconnect with jittered exponential backoff.

import type { ApiError, EventFrame, EventName, Events, Method, ParamsOf, ResponseFrame, ResultOf } from './protocol';

export type ConnState = 'connecting' | 'open' | 'reconnecting' | 'unauthorized' | 'closed';

export class RpcError extends Error implements ApiError {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
    this.name = 'RpcError';
  }
}

export interface CallOptions {
  /** Target host id; omitted = the home daemon. */
  host?: string;
  /** Milliseconds before the call rejects with `timeout`. */
  timeout?: number;
}

type Handler<E extends EventName> = (data: Events[E], host: string) => void;
type AnyHandler = (data: unknown, host: string) => void;

interface Pending {
  resolve: (value: unknown) => void;
  reject: (err: RpcError) => void;
  timer: number;
}

const DEFAULT_TIMEOUT = 20_000;
const BACKOFF_MIN = 400;
const BACKOFF_MAX = 10_000;

export class Client {
  readonly url: string;
  readonly token: string;
  state: ConnState = 'connecting';

  #ws: WebSocket | null = null;
  #nextId = 1;
  #pending = new Map<number, Pending>();
  #handlers = new Map<string, Set<AnyHandler>>();
  #stateListeners = new Set<(s: ConnState) => void>();
  #openListeners = new Set<() => void>();
  #attempt = 0;
  #retryTimer: number | undefined;
  #everOpened = false;
  #stopped = false;

  constructor(url: string, token: string) {
    this.url = url;
    this.token = token;
    if (typeof window !== 'undefined') {
      const wake = () => {
        if (this.state === 'reconnecting' && document.visibilityState === 'visible') this.#reconnectNow();
      };
      document.addEventListener('visibilitychange', wake);
      window.addEventListener('online', () => this.#reconnectNow());
    }
  }

  /** Opens the socket (idempotent). */
  connect(): void {
    if (this.#stopped || this.#ws) return;
    const sep = this.url.includes('?') ? '&' : '?';
    const ws = new WebSocket(`${this.url}${sep}token=${encodeURIComponent(this.token)}`);
    this.#ws = ws;
    this.#setState(this.#everOpened ? 'reconnecting' : 'connecting');
    let opened = false;

    ws.onopen = () => {
      opened = true;
      this.#everOpened = true;
      this.#attempt = 0;
      this.#setState('open');
      for (const fn of this.#openListeners) fn();
    };
    ws.onmessage = (ev) => this.#onFrame(ev.data);
    ws.onclose = (ev) => {
      if (this.#ws !== ws) return;
      this.#ws = null;
      this.#failPending(new RpcError('unavailable', 'connection lost'));
      if (this.#stopped) return this.#setState('closed');
      if (ev.code === 4401 || ev.code === 1008) return this.#setState('unauthorized');
      if (!opened) {
        // The upgrade was refused. If the daemon answers health checks, the token is the problem.
        void this.#probeAuth().then((authProblem) => {
          if (authProblem) this.#setState('unauthorized');
          else this.#scheduleRetry();
        });
        return;
      }
      this.#scheduleRetry();
    };
  }

  /** Stops the client for good (no reconnect). */
  close(): void {
    this.#stopped = true;
    clearTimeout(this.#retryTimer);
    this.#ws?.close();
    this.#ws = null;
    this.#setState('closed');
  }

  call<M extends Method>(method: M, params?: ParamsOf<M>, opts: CallOptions = {}): Promise<ResultOf<M>> {
    const ws = this.#ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      return Promise.reject(new RpcError('unavailable', 'not connected to the daemon'));
    }
    const id = this.#nextId++;
    const frame: Record<string, unknown> = { id, method };
    if (params !== undefined) frame.params = params;
    if (opts.host) frame.host = opts.host;
    return new Promise<ResultOf<M>>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new RpcError('timeout', `${method} timed out`));
      }, opts.timeout ?? DEFAULT_TIMEOUT);
      this.#pending.set(id, { resolve: resolve as (v: unknown) => void, reject, timer });
      ws.send(JSON.stringify(frame));
    });
  }

  on<E extends EventName>(event: E, handler: Handler<E>): () => void {
    let set = this.#handlers.get(event);
    if (!set) this.#handlers.set(event, (set = new Set()));
    set.add(handler as AnyHandler);
    return () => set.delete(handler as AnyHandler);
  }

  onState(fn: (s: ConnState) => void): () => void {
    this.#stateListeners.add(fn);
    return () => this.#stateListeners.delete(fn);
  }

  /** Runs on every (re)connect. */
  onOpen(fn: () => void): () => void {
    this.#openListeners.add(fn);
    return () => this.#openListeners.delete(fn);
  }

  /** HTTP origin of the daemon this client talks to. */
  get httpOrigin(): string {
    const u = new URL(this.url);
    return `${u.protocol === 'wss:' ? 'https:' : 'http:'}//${u.host}`;
  }

  #onFrame(raw: unknown) {
    if (typeof raw !== 'string') return;
    let msg: ResponseFrame & Partial<EventFrame>;
    try {
      msg = JSON.parse(raw);
    } catch {
      return;
    }
    if (typeof msg.event === 'string') {
      const set = this.#handlers.get(msg.event);
      if (!set) return;
      for (const fn of set) {
        try {
          fn(msg.data, msg.host ?? '');
        } catch (err) {
          console.error(`handler for ${msg.event} failed`, err);
        }
      }
      return;
    }
    const pending = this.#pending.get(msg.id);
    if (!pending) return;
    this.#pending.delete(msg.id);
    clearTimeout(pending.timer);
    if (msg.error) pending.reject(new RpcError(msg.error.code, msg.error.message));
    else pending.resolve(msg.result ?? {});
  }

  #failPending(err: RpcError) {
    for (const p of this.#pending.values()) {
      clearTimeout(p.timer);
      p.reject(err);
    }
    this.#pending.clear();
  }

  #scheduleRetry() {
    this.#setState('reconnecting');
    const base = Math.min(BACKOFF_MAX, BACKOFF_MIN * 2 ** this.#attempt++);
    const delay = base / 2 + Math.random() * (base / 2);
    clearTimeout(this.#retryTimer);
    this.#retryTimer = setTimeout(() => {
      this.#retryTimer = undefined;
      this.connect();
    }, delay);
  }

  #reconnectNow() {
    if (this.#ws || this.#stopped || this.state === 'unauthorized') return;
    clearTimeout(this.#retryTimer);
    this.#retryTimer = undefined;
    this.#attempt = 0;
    this.connect();
  }

  async #probeAuth(): Promise<boolean> {
    try {
      const res = await fetch(`${this.httpOrigin}/api/health`, { cache: 'no-store' });
      if (!res.ok) return false;
      const body = await res.json();
      return body?.ok === true;
    } catch {
      return false;
    }
  }

  #setState(s: ConnState) {
    if (this.state === s) return;
    this.state = s;
    for (const fn of this.#stateListeners) fn(s);
  }
}
