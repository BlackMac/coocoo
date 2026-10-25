<script lang="ts">
  import { untrack } from 'svelte';
  import { fleet, key } from '../lib/fleet.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { AskAnswer, AskUiPayload, UiRequest, UiResponse } from '../lib/protocol';
  import { MessageSquareText, Check, KeyRound } from '../lib/icons';
  import SecretRequest from './SecretRequest.svelte';

  let { request, host, compact = false }: { request: UiRequest; host: string; compact?: boolean } = $props();

  const p = $derived(request.payload);
  let busy = $state(false);
  let text = $state(untrack(() => (request.payload.method === 'editor' ? (request.payload.prefill ?? '') : '')));
  // ask: per-question selection + free text
  let picked = $state<Record<string, string[]>>({});
  let custom = $state<Record<string, string>>({});
  /** Compact cards reveal the free-text field on demand. */
  let otherOpen = $state<Record<string, boolean>>({});

  /** The request is answered: take it off the list now (`agent.requestResolved` follows). */
  function drop() {
    const list = fleet.requests[key(host, request.agentId)];
    const i = list?.findIndex((r) => r.id === request.id) ?? -1;
    if (list && i >= 0) list.splice(i, 1);
  }

  async function respond(response: UiResponse) {
    if (busy) return;
    busy = true;
    try {
      await fleet.call('agents.respond', { id: request.agentId, requestId: request.id, response }, host);
      drop();
    } catch (err) {
      ui.error('Could not send the answer', err);
    } finally {
      busy = false;
    }
  }

  function toggle(qid: string, label: string, multi: boolean) {
    const cur = picked[qid] ?? [];
    picked[qid] = multi ? (cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label]) : cur[0] === label ? [] : [label];
  }

  function submitAsk(payload: AskUiPayload) {
    const answers: AskAnswer[] = payload.questions.map((q) => {
      const c = custom[q.id]?.trim();
      return { id: q.id, selectedOptions: picked[q.id] ?? [], ...(c ? { customInput: c } : {}) };
    });
    void respond({ answers });
  }

  const askReady = $derived(
    p.method === 'ask' && p.questions.every((q) => (picked[q.id]?.length ?? 0) > 0 || !!custom[q.id]?.trim()),
  );

  function onTextKey(e: KeyboardEvent, multiline: boolean) {
    if (e.key === 'Enter' && (!multiline || e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void respond({ value: text });
    }
  }
</script>

<div class="req" class:compact>
  <div class="head">
    {#if p.method === 'secret'}<KeyRound size={14} />{:else}<MessageSquareText size={14} />{/if}
    <span class="kind">{p.method === 'secret' ? 'Secret requested' : p.method === 'ask' ? (p.questions.length > 1 ? `${p.questions.length} questions` : 'Question') : p.method === 'confirm' ? 'Confirmation' : 'Input requested'}</span>
    <button class="btn ghost sm dismiss" disabled={busy} onclick={() => respond({ cancelled: true })}>{p.method === 'secret' ? 'Decline' : 'Dismiss'}</button>
  </div>

  {#if p.method === 'secret'}
    <SecretRequest {request} {host} {compact} ondone={drop} />
  {:else if p.method === 'ask'}
    {#each p.questions as q (q.id)}
      <fieldset class="q">
        <legend>
          {#if q.header}<span class="chip">{q.header}</span>{/if}
          <span class="qt">{q.question}</span>
          {#if q.multi}<span class="faint">· choose any</span>{/if}
        </legend>
        <div class="opts">
          {#each q.options as o, i (o.label)}
            {@const on = picked[q.id]?.includes(o.label)}
            <button type="button" class="opt" class:on aria-pressed={on} onclick={() => toggle(q.id, o.label, !!q.multi)}>
              <span class="mark" class:multi={q.multi}>{#if on}<Check size={11} strokeWidth={3} />{/if}</span>
              <span class="ot">
                <span class="ol">{o.label}{#if q.recommended === i}<span class="rec">Recommended</span>{/if}</span>
                {#if o.description && !compact}<span class="od">{o.description}</span>{/if}
              </span>
            </button>
          {/each}
          {#if compact && !otherOpen[q.id]}
            <button type="button" class="opt" onclick={() => (otherOpen[q.id] = true)}><span class="ot faint">Other…</span></button>
          {/if}
        </div>
        {#if !compact || otherOpen[q.id]}
          <!-- svelte-ignore a11y_autofocus -->
          <input class="input other" placeholder="Other — type your own answer" bind:value={custom[q.id]} autofocus={compact} />
        {/if}
      </fieldset>
    {/each}
    <div class="actions">
      <button class="btn primary sm" disabled={!askReady || busy} onclick={() => submitAsk(p as AskUiPayload)}>Send answers</button>
    </div>
  {:else if p.method === 'select'}
    <div class="title">{p.title}</div>
    <div class="opts">
      {#each p.options as o, i (o)}
        <button type="button" class="opt" disabled={busy} onclick={() => respond({ value: o })}>
          <span class="ot">
            <span class="ol">{o}</span>
            {#if p.optionDetails?.[i]?.description && !compact}<span class="od">{p.optionDetails[i].description}</span>{/if}
          </span>
        </button>
      {/each}
    </div>
  {:else if p.method === 'confirm'}
    <div class="title">{p.title}</div>
    {#if p.message}<div class="msg">{p.message}</div>{/if}
    <div class="actions">
      <button class="btn sm" disabled={busy} onclick={() => respond({ confirmed: false })}>No</button>
      <button class="btn primary sm" disabled={busy} onclick={() => respond({ confirmed: true })}>Yes</button>
    </div>
  {:else if p.method === 'input'}
    <div class="title">{p.title}</div>
    <div class="inline">
      <input class="input" bind:value={text} placeholder={p.placeholder ?? ''} onkeydown={(e) => onTextKey(e, false)} />
      <button class="btn primary sm" disabled={busy} onclick={() => respond({ value: text })}>Send</button>
    </div>
  {:else if p.method === 'editor'}
    <div class="title">{p.title}</div>
    <textarea class="textarea mono" rows={compact ? 3 : 8} bind:value={text} onkeydown={(e) => onTextKey(e, true)}></textarea>
    <div class="actions">
      <span class="hint">⌘↵ to send</span>
      <button class="btn primary sm" disabled={busy} onclick={() => respond({ value: text })}>Send</button>
    </div>
  {/if}
</div>

<style>
  .req {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border: 1px solid color-mix(in srgb, var(--amber) 35%, var(--border));
    border-radius: var(--r-lg);
    background: color-mix(in srgb, var(--amber) 5%, var(--bg-1));
  }
  .req.compact {
    padding: 2px 0 2px 12px;
    gap: 8px;
    border: 0;
    border-left: 2px solid color-mix(in srgb, var(--amber) 45%, transparent);
    border-radius: 0;
    background: transparent;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--amber);
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .dismiss {
    margin-left: auto;
    text-transform: none;
    letter-spacing: 0;
  }
  .title {
    font-weight: 500;
  }
  .msg {
    color: var(--text-2);
    white-space: pre-wrap;
  }
  fieldset {
    margin: 0;
    padding: 0;
    border: 0;
    display: flex;
    flex-direction: column;
    gap: 7px;
    min-width: 0;
  }
  legend {
    display: flex;
    align-items: baseline;
    gap: 6px;
    flex-wrap: wrap;
    padding: 0;
    margin-bottom: 7px;
  }
  .qt {
    font-weight: 500;
  }
  .opts {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .compact .opts {
    flex-direction: row;
    flex-wrap: wrap;
  }
  .opt {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    padding: 7px 10px;
    border: 1px solid var(--border-strong);
    border-radius: var(--r);
    background: var(--bg);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition:
      border-color var(--t-fast) var(--ease),
      background var(--t-fast) var(--ease);
  }
  .compact .opt {
    padding: 4px 9px;
    align-items: center;
  }
  .opt:hover:not(:disabled) {
    border-color: var(--text-3);
  }
  .opt.on {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .mark {
    display: grid;
    place-items: center;
    flex: none;
    width: 14px;
    height: 14px;
    margin-top: 2px;
    border: 1.5px solid var(--text-3);
    border-radius: 50%;
    color: #fff;
  }
  .compact .mark {
    margin-top: 0;
  }
  .mark.multi {
    border-radius: 3px;
  }
  .on .mark {
    border-color: var(--accent);
    background: var(--accent);
  }
  .ot {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .ol {
    display: flex;
    align-items: center;
    gap: 6px;
    font-weight: 500;
  }
  .od {
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .rec {
    padding: 0 5px;
    border-radius: 99px;
    background: var(--green-soft);
    color: var(--green);
    font-size: 10px;
    font-weight: 600;
  }
  .other {
    height: 28px;
  }
  .actions,
  .inline {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 6px;
  }
  .actions .hint {
    margin-right: auto;
  }
  .inline .input {
    flex: 1;
  }
</style>
