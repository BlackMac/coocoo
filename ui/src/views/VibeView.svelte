<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { fleet, key } from '../lib/fleet.svelte';
  import type { AgentSummary, AskQuestion, TimelineItem, UiResponse } from '../lib/protocol';
  import { agentState, STATE_RANK, type AgentState } from '../lib/status';
  import { href, router } from '../lib/router.svelte';
  import { ui } from '../lib/ui.svelte';
  import { now } from '../lib/clock.svelte';
  import { ago, compact, cost, duration } from '../lib/format';
  import { renderMarkdown } from '../lib/markdown';
  import { parseError } from '../lib/providerError';
  import { Dictation } from '../lib/speech.svelte';
  import { track } from '../lib/track';
  import { ArrowLeft, Ban, Check, ChevronDown, CornerDownLeft, Mic } from '../lib/icons';

  // A controller-style display for the whole fleet, after SlashVibe: a list of sessions with their
  // state, one session's latest round, its pending question as lettered options, and prompts by
  // voice. Driven by a joystick (arrows / hjkl), ↵, Esc and push-to-talk (Space), on screen or
  // on the keyboard.

  type Screen = 'list' | 'detail' | 'options';
  type Button = 'up' | 'down' | 'left' | 'right' | 'enter' | 'back';

  const LABEL: Record<AgentState, string> = {
    needs: 'WAITING',
    error: 'ERROR',
    working: 'WORKING',
    review: 'DONE',
    idle: 'IDLE',
    hibernated: 'ASLEEP',
    archived: 'ARCHIVED',
  };
  const KEYS: Record<string, Button> = {
    ArrowUp: 'up',
    k: 'up',
    ArrowDown: 'down',
    j: 'down',
    ArrowLeft: 'left',
    h: 'left',
    ArrowRight: 'right',
    l: 'right',
    Enter: 'enter',
    Escape: 'back',
    Backspace: 'back',
  };
  const LETTERS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ';
  /** Held longer than this, the talk key is push-to-talk; a shorter tap toggles listening. */
  const HOLD_MS = 350;

  const akey = (a: AgentSummary) => key(a.hostId, a.id);

  // Top-level agents in triage order: waiting and failed ones first.
  const agents = $derived(
    fleet.agentList
      .filter((a) => !a.parentId && a.status !== 'stopped')
      .sort((x, y) => STATE_RANK[agentState(x)] - STATE_RANK[agentState(y)] || y.lastActivityAt - x.lastActivityAt),
  );
  const counts = $derived.by(() => {
    const c = { needs: 0, working: 0, review: 0 };
    for (const a of agents) {
      const s = agentState(a);
      if (s === 'needs' || s === 'error') c.needs++;
      else if (s === 'working') c.working++;
      else if (s === 'review') c.review++;
    }
    return c;
  });
  const spend = $derived(agents.reduce((s, a) => s + (a.costUsd ?? 0), 0));
  const clock = $derived(new Date(now.value).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }));

  let screen = $state<Screen>('list');
  let selKey = $state<string | null>(null);
  const selIdx = $derived(agents.findIndex((a) => akey(a) === selKey));
  const sel = $derived<AgentSummary | undefined>(agents[selIdx]);
  const selState = $derived<AgentState>(sel ? agentState(sel) : 'idle');
  const req = $derived(sel ? fleet.requests[akey(sel)]?.[0] : undefined);

  // The selection always sits on a listed agent; when it disappears, back to the top of the list.
  $effect(() => {
    if (agents.some((a) => akey(a) === selKey)) return;
    selKey = agents[0] ? akey(agents[0]) : null;
    screen = 'list';
    untrack(closeSheet);
  });

  // ─── Latest round of the selected agent ───────────────────────────────────────────────────────

  const tl = $derived(sel && screen !== 'list' ? fleet.timeline(sel.hostId, sel.id) : null);
  const round = $derived.by(() => {
    let reply: TimelineItem | undefined;
    let prompt: TimelineItem | undefined;
    let activity: string | undefined;
    const items = tl?.items ?? [];
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === 'user') {
        prompt = it;
        break;
      }
      if (!reply && it.kind === 'assistant' && it.text?.trim()) reply = it;
      if (!activity && !reply && it.kind === 'tool' && it.tool?.status === 'running') activity = it.tool.intent ?? it.tool.name;
    }
    return { reply, prompt, activity };
  });
  const meta = $derived.by(() => {
    if (!sel) return '';
    const parts: string[] = [];
    if (round.prompt) {
      const end = selState === 'working' ? now.value : (round.reply?.ts ?? sel.lastActivityAt);
      const d = duration(Math.max(0, end - round.prompt.ts));
      parts.push(selState === 'working' ? `for ${d}` : `Worked for ${d}`);
    }
    if (sel.tokens) parts.push(`${compact(sel.tokens)} tokens`);
    if (sel.costUsd) parts.push(cost(sel.costUsd));
    if (selState !== 'working') parts.push(`${ago(sel.lastActivityAt, now.value)} ago`);
    return parts.join(' · ');
  });

  // ─── Pending request as lettered options ──────────────────────────────────────────────────────

  interface Row {
    label: string;
    detail?: string;
    on?: boolean;
    rec?: boolean;
    /** The closing "Done" row of a multi-select question. */
    done?: boolean;
    act: () => void;
  }

  let oi = $state(0);
  /** `ask`: the question on screen; its answers are collected until the last one is done. */
  let qi = $state(0);
  let picked = $state<Record<string, string[]>>({});
  let custom = $state<Record<string, string>>({});
  let busy = $state(false);

  let shownReq = '';
  $effect(() => {
    const r = req;
    if ((r?.id ?? '') === shownReq) return;
    shownReq = r?.id ?? '';
    qi = 0;
    picked = {};
    custom = {};
    oi = r?.payload.method === 'ask' ? (r.payload.questions[0]?.recommended ?? 0) : 0;
  });
  // Answered here or elsewhere: show what the agent does next.
  $effect(() => {
    if (screen === 'options' && !req) screen = 'detail';
  });

  const opts = $derived.by((): { title: string; sub?: string; step?: string; rows: Row[] } | null => {
    if (!req) return null;
    const p = req.payload;
    switch (p.method) {
      case 'ask': {
        const q = p.questions[qi];
        if (!q) return null;
        const rows: Row[] = q.options.map((o, i) => ({
          label: o.label,
          detail: o.description,
          on: picked[q.id]?.includes(o.label),
          rec: q.recommended === i,
          act: () => choose(p.questions, q, o.label),
        }));
        if (q.multi) rows.push({ label: 'Done', done: true, act: () => advance(p.questions) });
        return {
          title: q.question,
          sub: [q.header, custom[q.id] ? `Your answer: ${custom[q.id]}` : ''].filter(Boolean).join(' · ') || undefined,
          step: p.questions.length > 1 ? `${qi + 1}/${p.questions.length}` : undefined,
          rows,
        };
      }
      case 'select':
        return { title: p.title, rows: p.options.map((o, i) => ({ label: o, detail: p.optionDetails?.[i]?.description, act: () => answer({ value: o }) })) };
      case 'confirm':
        return {
          title: p.title,
          sub: p.message,
          rows: [
            { label: 'Yes', act: () => answer({ confirmed: true }) },
            { label: 'No', act: () => answer({ confirmed: false }) },
          ],
        };
      case 'input':
      case 'editor': {
        const prefill = p.method === 'editor' ? (p.prefill ?? '') : '';
        return { title: p.title, rows: [{ label: dictation.supported ? 'Speak or type the answer' : 'Type the answer', act: () => openSheet('answer', prefill) }] };
      }
      case 'secret':
        return { title: `Secret ${p.name} requested`, sub: p.description, rows: [{ label: 'Enter it in Coocoo', act: openAgent }] };
    }
    return null;
  });
  const cur = $derived(Math.max(0, Math.min(oi, (opts?.rows.length ?? 1) - 1)));

  function choose(qs: AskQuestion[], q: AskQuestion, label: string) {
    const was = picked[q.id] ?? [];
    if (q.multi) {
      picked[q.id] = was.includes(label) ? was.filter((l) => l !== label) : [...was, label];
      return;
    }
    picked[q.id] = [label];
    advance(qs);
  }

  /** Next question, or send all answers after the last one. */
  function advance(qs: AskQuestion[]) {
    const q = qs[qi];
    if (!picked[q.id]?.length && !custom[q.id]?.trim()) return;
    if (qi < qs.length - 1) {
      qi++;
      oi = qs[qi].recommended ?? 0;
      return;
    }
    void answer({
      answers: qs.map((x) => {
        const c = custom[x.id]?.trim();
        return { id: x.id, selectedOptions: picked[x.id] ?? [], ...(c ? { customInput: c } : {}) };
      }),
    });
  }

  async function answer(response: UiResponse) {
    const a = sel;
    const r = req;
    if (!a || !r || busy) return;
    busy = true;
    try {
      await fleet.respond(a.hostId, r, response);
      track('vibe', { agentId: a.id, meta: { action: 'answer' } });
      flash('Answer sent');
    } catch (err) {
      ui.error('Could not send the answer', err);
    } finally {
      busy = false;
    }
  }

  function openAgent() {
    if (sel) router.go({ name: 'agent', host: sel.hostId, id: sel.id });
  }

  // ─── Prompt sheet and voice ───────────────────────────────────────────────────────────────────

  const dictation = new Dictation();
  /** `agentKey` pins the target: the list may reorder while the sheet is open. */
  let sheet = $state<{ purpose: 'prompt' | 'answer'; text: string; agentKey: string; voice: boolean } | null>(null);
  let ta = $state<HTMLTextAreaElement>();
  let pressedAt = 0;

  function openSheet(purpose: 'prompt' | 'answer', text = '') {
    if (!sel) return;
    sheet = { purpose, text, agentKey: akey(sel), voice: false };
    // Without dictation the sheet is a text field; with it, the keyboard stays closed on phones.
    if (!dictation.supported) void tick().then(() => ta?.focus());
  }

  function closeSheet() {
    dictation.cancel();
    sheet = null;
  }

  function talkDown() {
    if (dictation.listening) {
      dictation.stop();
      pressedAt = 0;
      return;
    }
    if (!sheet) {
      const p = screen === 'options' ? req?.payload : undefined;
      if (p && (p.method === 'ask' || p.method === 'input' || p.method === 'editor')) openSheet('answer');
      else if (p) return;
      else openSheet('prompt');
    }
    if (!sheet || !dictation.supported) return;
    sheet.voice = true;
    pressedAt = Date.now();
    dictation.start((t) => {
      if (sheet && t) sheet.text = sheet.text.trim() ? `${sheet.text.trimEnd()} ${t}` : t;
    });
  }

  function talkUp() {
    if (pressedAt && Date.now() - pressedAt > HOLD_MS) dictation.stop();
    pressedAt = 0;
  }

  async function submitSheet() {
    if (!sheet) return;
    // Words still being recognized count too: ↵ while talking sends what is on screen.
    const text = [sheet.text.trim(), dictation.interim].filter(Boolean).join(' ').trim();
    if (!text) return;
    const { purpose, agentKey, voice } = sheet;
    closeSheet();
    const a = fleet.agents[agentKey];
    if (!a) return;
    if (purpose === 'prompt') {
      const running = a.status === 'running' || a.status === 'starting' || a.status === 'waiting';
      fleet.sendPrompt(a.hostId, a.id, { message: text, images: [], draft: { text, images: [], elements: [] } }, running ? 'steer' : 'auto');
      track('vibe', { agentId: a.id, meta: { action: 'prompt', voice } });
      flash(running ? 'Sent as steering' : 'Sent');
      return;
    }
    const p = req?.payload;
    if (p?.method === 'ask') {
      custom[p.questions[qi].id] = text;
      advance(p.questions);
    } else await answer({ value: text });
  }

  let toast = $state<string | null>(null);
  let toastTimer: number | undefined;
  function flash(msg: string) {
    toast = msg;
    clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => (toast = null), 1600);
  }

  // ─── Input ────────────────────────────────────────────────────────────────────────────────────

  let screenEl = $state<HTMLElement>();
  let bodyEl = $state<HTMLElement>();

  function open() {
    if (sel) screen = req ? 'options' : 'detail';
  }

  function press(b: Button) {
    if (sheet) {
      if (b === 'enter') void submitSheet();
      else if (b === 'back') closeSheet();
      return;
    }
    const step = b === 'up' ? -1 : b === 'down' ? 1 : 0;
    if (screen === 'list') {
      if (step && agents.length) selKey = akey(agents[Math.max(0, Math.min(agents.length - 1, selIdx + step))]);
      else if (b === 'enter' || b === 'right') open();
    } else if (screen === 'detail') {
      if (step) bodyEl?.scrollBy({ top: step * 90, behavior: 'smooth' });
      else if (b === 'left' || b === 'back') screen = 'list';
      else if (b === 'right' && req) screen = 'options';
      else if (b === 'enter') {
        if (req) screen = 'options';
        else openAgent();
      }
    } else if (opts && !busy) {
      if (step) oi = Math.max(0, Math.min(opts.rows.length - 1, cur + step));
      else if (b === 'enter' || b === 'right') opts.rows[cur]?.act();
      else if (qi > 0) {
        qi--;
        oi = 0;
      } else screen = 'detail';
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.metaKey || e.ctrlKey || e.altKey || ui.dialog || ui.paletteOpen) return;
    const t = e.target as HTMLElement;
    if (t === ta) {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        void submitSheet();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        closeSheet();
      }
      return;
    }
    if (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName)) return;
    if (e.key === ' ') {
      e.preventDefault();
      if (!e.repeat) talkDown();
      return;
    }
    const b = KEYS[e.key];
    if (b) {
      e.preventDefault();
      press(b);
    } else if (screen === 'options' && !sheet && /^[1-9]$/.test(e.key) && opts?.rows[Number(e.key) - 1] && !busy) {
      oi = Number(e.key) - 1;
      opts.rows[oi].act();
    }
  }

  function onkeyup(e: KeyboardEvent) {
    if (e.key === ' ' && e.target !== ta) talkUp();
  }

  // Keep the selected row or option in view.
  $effect(() => {
    void [selIdx, cur, screen];
    void tick().then(() => screenEl?.querySelector('.sel')?.scrollIntoView({ block: 'nearest' }));
  });

  $effect(() => () => {
    dictation.cancel();
    clearTimeout(toastTimer);
  });
</script>

<svelte:window {onkeydown} {onkeyup} />

<div class="vibe">
  <header class="top">
    <a class="back" href={href({ name: 'overview' })}><ArrowLeft size={14} /> Coocoo</a>
    <span class="hints">↑↓ move · ↵ select · Esc back · hold Space to talk</span>
  </header>

  <div class="device">
    <div class="screen" bind:this={screenEl}>
      {#if screen === 'list' || !sel}
        <div class="status num">
          {#if fleet.conn !== 'open'}<span class="offline">OFFLINE</span>{/if}
          <span data-state="needs"><i></i>{counts.needs}</span>
          <span data-state="working"><i></i>{counts.working}</span>
          <span data-state="review"><i></i>{counts.review}</span>
          <span class="grow"></span>
          <span class="spend">{cost(spend)}</span>
          <span>{clock}</span>
        </div>
        <div class="rows">
          {#each agents as a, i (akey(a))}
            {@const s = agentState(a)}
            <button
              class="row"
              class:sel={i === selIdx}
              data-state={s}
              tabindex="-1"
              onclick={() => {
                selKey = akey(a);
                open();
              }}
            >
              <span class="pill"><i></i>{LABEL[s]}</span>
              <span class="title ellipsis">{a.title}</span>
              <span class="when num">{ago(a.lastActivityAt, now.value)}</span>
            </button>
          {:else}
            <div class="empty">No agents yet.<span>Press N to start one.</span></div>
          {/each}
        </div>
      {:else}
        <div class="head" data-state={selState}>
          <i></i><span class="ellipsis">{sel.title}</span>
          {#if screen === 'options' && opts?.step}<span class="step num">{opts.step}</span>{/if}
        </div>
        {#if screen === 'detail'}
          <div class="body" bind:this={bodyEl}>
            <div class="meta" data-state={selState}>
              <strong>{selState === 'working' ? 'Working' : 'This round'}</strong>
              <span>{meta}</span>
            </div>
            {#if round.prompt?.text}<div class="prompt">› {round.prompt.text}</div>{/if}
            {#if selState === 'working'}
              <div class="activity" data-state="working">
                <i></i>{round.activity ?? 'Thinking…'}
                {#if sel.todos?.total}<span class="faint num">· step {Math.min(sel.todos.done + 1, sel.todos.total)} of {sel.todos.total}</span>{/if}
              </div>
            {/if}
            {#if selState === 'error' && sel.error}
              <div class="error">{parseError(sel.error).title}</div>
            {/if}
            {#if round.reply?.text}
              <div class="md reply">{@html renderMarkdown(round.reply.text, round.reply.partial ? undefined : round.reply.id)}</div>
            {:else if tl && !tl.loaded}
              <div class="faint">Loading…</div>
            {:else if selState !== 'working'}
              <div class="faint">No answer in this round.</div>
            {/if}
          </div>
          <div class="foot">
            {#if req}<span data-state="needs"><i></i>Waiting for you</span><span class="grow"></span><span>↵ answer</span>
            {:else}<span class="grow"></span><span>↵ open in Coocoo · hold Space to prompt</span>{/if}
          </div>
        {:else if opts}
          <div class="opts">
            <div class="q">
              <div class="qt">{opts.title}</div>
              {#if opts.sub}<div class="qs">{opts.sub}</div>{/if}
            </div>
            {#each opts.rows as r, i (i)}
              <button
                class="opt"
                class:sel={i === cur}
                class:on={r.on}
                tabindex="-1"
                disabled={busy}
                onclick={() => {
                  oi = i;
                  r.act();
                }}
              >
                <span class="letter">{r.done ? '↵' : LETTERS[i]}</span>
                <span class="ot">
                  <span class="ol">{r.label}{#if r.rec}<em>recommended</em>{/if}</span>
                  {#if r.detail}<span class="od">{r.detail}</span>{/if}
                </span>
                {#if r.on}<Check size={16} strokeWidth={3} />{/if}
              </button>
            {/each}
          </div>
        {/if}
      {/if}

      {#if sheet}
        <div class="sheet">
          <div class="sh-head">
            <span class="led" class:live={dictation.listening}></span>
            <span class="ellipsis">
              {#if dictation.listening}Listening…{:else if sheet.purpose === 'answer'}Your answer{:else}To {fleet.agents[sheet.agentKey]?.title ?? 'agent'}{/if}
            </span>
          </div>
          <textarea
            bind:this={ta}
            bind:value={sheet.text}
            rows="4"
            placeholder={dictation.supported ? 'Speak, or tap here to type' : 'Type your prompt'}
          ></textarea>
          {#if dictation.interim}<div class="interim">{dictation.interim}</div>{/if}
          {#if dictation.error}<div class="error">{dictation.error}</div>{/if}
          <div class="sh-foot"><span>↵ send</span><span>Esc discard</span></div>
        </div>
      {/if}
      {#if toast}<div class="toast">{toast}</div>{/if}
    </div>

    <div class="controls">
      <div class="keys">
        <button class="key" tabindex="-1" aria-label="Back" onclick={() => press('back')}><Ban size={18} /><span>Esc</span></button>
        <button class="key" tabindex="-1" aria-label="Enter" onclick={() => press('enter')}><CornerDownLeft size={18} /><span>↵</span></button>
        <button
          class="key"
          class:live={dictation.listening}
          tabindex="-1"
          aria-label={dictation.supported ? 'Talk' : 'Type a prompt'}
          title={dictation.supported ? 'Hold to talk, tap to start and stop' : 'Voice input is not available in this browser'}
          onpointerdown={talkDown}
          onpointerup={talkUp}
          onpointercancel={talkUp}
          oncontextmenu={(e) => e.preventDefault()}
        >
          <Mic size={18} /><span>Space</span>
        </button>
      </div>
      <div class="stick">
        <button class="dir up" tabindex="-1" aria-label="Up" onclick={() => press('up')}><ChevronDown size={16} /></button>
        <button class="dir right" tabindex="-1" aria-label="Right" onclick={() => press('right')}><ChevronDown size={16} /></button>
        <button class="dir down" tabindex="-1" aria-label="Down" onclick={() => press('down')}><ChevronDown size={16} /></button>
        <button class="dir left" tabindex="-1" aria-label="Left" onclick={() => press('left')}><ChevronDown size={16} /></button>
        <button class="cap" tabindex="-1" aria-label="Select" onclick={() => press('enter')}></button>
        <span class="led" class:live={dictation.listening}></span>
      </div>
    </div>
  </div>
</div>

<style>
  .vibe {
    --y: #ffd23f;
    --g: #3ddc6a;
    --b: #5c9dff;
    --r: #ff5b4f;
    --n: #8b9099;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 18px;
    padding: 24px;
    box-sizing: border-box;
    overflow: hidden;
    background: radial-gradient(ellipse at 50% 30%, var(--bg-2), var(--bg) 70%);
  }
  [data-state] {
    --c: var(--n);
  }
  [data-state='needs'] {
    --c: var(--y);
  }
  [data-state='error'] {
    --c: var(--r);
  }
  [data-state='working'] {
    --c: var(--b);
  }
  [data-state='review'] {
    --c: var(--g);
  }
  [data-state='hibernated'] {
    --c: #5d626b;
  }

  .top {
    width: min(600px, 100%);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: var(--fs-sm);
    color: var(--text-3);
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    text-decoration: none;
  }
  .back:hover {
    color: var(--text);
  }

  /* ─── Device body ─── */
  .device {
    width: min(600px, 100%);
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px;
    box-sizing: border-box;
    border-radius: 28px;
    background: linear-gradient(180deg, #e6e7e9, #bfc1c5);
    box-shadow:
      0 40px 80px -20px rgba(0, 0, 0, 0.55),
      inset 0 1px 0 rgba(255, 255, 255, 0.8),
      inset 0 -3px 0 rgba(0, 0, 0, 0.12);
  }
  .screen {
    position: relative;
    height: min(430px, calc(100dvh - 300px));
    min-height: 240px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 10px solid #0c0c0e;
    border-radius: 18px;
    background: #000;
    color: #f3f3f5;
    font-family: var(--font);
    font-size: 15px;
    line-height: 1.45;
    box-shadow:
      inset 0 0 0 1px #1d1d20,
      0 1px 0 rgba(255, 255, 255, 0.6);
  }
  .grow {
    flex: 1;
  }
  .faint {
    color: #80858e;
  }
  i {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--c);
    box-shadow: 0 0 8px var(--c);
  }
  [data-state='working'] > i {
    animation: breathe 1.6s ease-in-out infinite;
  }
  @keyframes breathe {
    50% {
      opacity: 0.35;
      box-shadow: none;
    }
  }

  /* ─── List ─── */
  .status {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 14px;
    background: #121214;
    font-family: var(--mono);
    font-size: 13px;
    color: #c9ccd2;
  }
  .status > span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .status .spend {
    color: #f3f3f5;
  }
  .offline {
    color: var(--y);
  }
  .rows,
  .opts,
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: #3a3c41 transparent;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
  }
  .row,
  .opt {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    flex: none;
    border: 2px solid transparent;
    border-radius: 10px;
    background: #141416;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      border-color var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
  }
  .row {
    padding: 9px 12px;
  }
  .row.sel {
    border-color: var(--c);
    box-shadow:
      0 0 14px color-mix(in srgb, var(--c) 45%, transparent),
      inset 0 0 10px color-mix(in srgb, var(--c) 20%, transparent);
  }
  .row.sel .title {
    font-weight: 600;
  }
  .pill {
    flex: none;
    width: 92px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: var(--c);
  }
  .pill i {
    width: 7px;
    height: 7px;
  }
  .title {
    flex: 1;
    min-width: 0;
  }
  .when {
    flex: none;
    font-size: 12px;
    color: #6f747d;
  }
  .empty {
    margin: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    color: #c9ccd2;
  }
  .empty span {
    color: #80858e;
    font-size: 13px;
  }

  /* ─── Detail ─── */
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 11px 14px;
    background: #121214;
    font-weight: 700;
  }
  .head .ellipsis {
    flex: 1;
  }
  .step {
    flex: none;
    font-family: var(--mono);
    font-size: 12px;
    color: #9aa0a8;
  }
  .body {
    padding: 12px 16px 16px;
    scrollbar-color: var(--g) transparent;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 10px;
    margin-bottom: 8px;
    font-size: 14px;
    color: #9aa0a8;
  }
  .meta strong {
    color: var(--c);
    font-size: 16px;
  }
  .prompt {
    margin-bottom: 10px;
    color: #9aa0a8;
    font-size: 14px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    white-space: pre-wrap;
  }
  .activity {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
    color: #dfe2e7;
    font-size: 14px;
  }
  .error {
    margin-bottom: 10px;
    color: var(--r);
    font-size: 14px;
  }
  .reply {
    font-size: 16px;
    line-height: 1.55;
    --text: #f3f3f5;
    --text-2: #b4b8bf;
    --bg-2: #18181b;
    --bg-3: #222226;
    --border: #2a2b2f;
    --code-bg: #111113;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 14px;
    border-top: 1px solid #1c1c1f;
    font-family: var(--mono);
    font-size: 11.5px;
    color: #80858e;
  }
  .foot [data-state] {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    color: var(--c);
  }

  /* ─── Options ─── */
  .opts {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    scrollbar-color: var(--y) transparent;
  }
  .q {
    padding: 2px 6px 4px;
  }
  .qt {
    font-weight: 600;
  }
  .qs {
    margin-top: 2px;
    font-size: 13px;
    color: #9aa0a8;
  }
  .opt {
    padding: 12px 14px;
  }
  .opt.sel {
    border-color: var(--y);
    box-shadow:
      0 0 16px color-mix(in srgb, var(--y) 45%, transparent),
      inset 0 0 10px color-mix(in srgb, var(--y) 18%, transparent);
  }
  .opt.on {
    color: var(--y);
  }
  .opt:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .letter {
    flex: none;
    width: 14px;
    font-family: var(--mono);
    font-weight: 700;
    color: var(--y);
  }
  .ot {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .ol em {
    margin-left: 8px;
    font-style: normal;
    font-family: var(--mono);
    font-size: 10.5px;
    letter-spacing: 0.04em;
    color: var(--g);
    text-transform: uppercase;
  }
  .od {
    font-size: 13px;
    color: #9aa0a8;
  }

  /* ─── Prompt sheet ─── */
  .sheet {
    position: absolute;
    inset: auto 0 0 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    border-top: 1px solid #2a2b2f;
    background: #0c0c0e;
    box-shadow: 0 -20px 40px rgba(0, 0, 0, 0.6);
  }
  .sh-head {
    display: flex;
    align-items: center;
    gap: 9px;
    font-weight: 600;
  }
  .sheet textarea {
    width: 100%;
    box-sizing: border-box;
    resize: none;
    padding: 8px 10px;
    border: 1px solid #2a2b2f;
    border-radius: 8px;
    background: #141416;
    color: inherit;
    font: inherit;
    outline: none;
  }
  .sheet textarea:focus {
    border-color: var(--g);
  }
  .interim {
    color: #9aa0a8;
    font-style: italic;
  }
  .sh-foot {
    display: flex;
    justify-content: space-between;
    font-family: var(--mono);
    font-size: 11.5px;
    color: #80858e;
  }
  .led {
    flex: none;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #3a3c41;
  }
  .led.live {
    background: var(--g);
    box-shadow: 0 0 10px var(--g);
    animation: breathe 1.1s ease-in-out infinite;
  }
  .toast {
    position: absolute;
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    padding: 6px 14px;
    border-radius: 999px;
    background: var(--g);
    color: #04140a;
    font-size: 13px;
    font-weight: 700;
    box-shadow: 0 0 18px color-mix(in srgb, var(--g) 50%, transparent);
  }

  /* ─── Controls ─── */
  .controls {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .keys {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  .key {
    height: 76px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border: 0;
    border-radius: 12px;
    background: linear-gradient(180deg, #d9dadd, #bbbdc1);
    color: #3b3d42;
    font-family: var(--mono);
    font-size: 10.5px;
    cursor: pointer;
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.7),
      0 3px 0 #9c9ea3,
      0 5px 10px rgba(0, 0, 0, 0.2);
    transition:
      transform 60ms ease,
      box-shadow 60ms ease;
  }
  .key span {
    opacity: 0.6;
  }
  .key:active,
  .key.live {
    transform: translateY(2px);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.5),
      0 1px 0 #9c9ea3,
      0 2px 4px rgba(0, 0, 0, 0.2);
  }
  .key.live {
    color: #138a3a;
  }
  .stick {
    position: relative;
    flex: none;
    width: 140px;
    height: 140px;
    border-radius: 50%;
    background: radial-gradient(circle at 50% 35%, #eceded, #c2c4c8);
    box-shadow:
      inset 0 2px 3px rgba(255, 255, 255, 0.8),
      inset 0 -3px 6px rgba(0, 0, 0, 0.15),
      0 4px 10px rgba(0, 0, 0, 0.2);
  }
  .dir {
    position: absolute;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    padding: 0;
    border: 0;
    background: none;
    color: #6b6e74;
    cursor: pointer;
  }
  .dir:active {
    color: #222;
  }
  .dir.up {
    top: 0;
    left: 50px;
    transform: rotate(180deg);
  }
  .dir.down {
    bottom: 0;
    left: 50px;
  }
  .dir.left {
    left: 0;
    top: 50px;
    transform: rotate(90deg);
  }
  .dir.right {
    right: 0;
    top: 50px;
    transform: rotate(-90deg);
  }
  .cap {
    position: absolute;
    inset: 38px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background:
      radial-gradient(circle, rgba(0, 0, 0, 0.12) 1px, transparent 1.5px) 0 0 / 6px 6px,
      radial-gradient(circle at 50% 35%, #f4f4f5, #cfd1d4);
    box-shadow:
      inset 0 1px 1px rgba(255, 255, 255, 0.9),
      0 2px 5px rgba(0, 0, 0, 0.25);
    cursor: pointer;
  }
  .cap:active {
    transform: scale(0.96);
  }
  .stick .led {
    position: absolute;
    right: 4px;
    bottom: 10px;
    width: 14px;
    height: 7px;
    border-radius: 4px;
  }

  @media (max-width: 640px), (max-height: 560px) {
    .vibe {
      padding: 0;
      gap: 0;
      justify-content: stretch;
    }
    .top {
      padding: 10px 14px;
    }
    .hints {
      display: none;
    }
    .device {
      flex: 1;
      min-height: 0;
      border-radius: 0;
      padding: 10px;
      gap: 12px;
    }
    .screen {
      flex: 1;
      height: auto;
      min-height: 0;
    }
    .key {
      height: 64px;
    }
    .stick {
      width: 120px;
      height: 120px;
    }
    .dir.up,
    .dir.down {
      left: 40px;
    }
    .dir.left,
    .dir.right {
      top: 40px;
    }
    .cap {
      inset: 32px;
    }
  }
  @media (hover: none) {
    .key span {
      display: none;
    }
  }
</style>
