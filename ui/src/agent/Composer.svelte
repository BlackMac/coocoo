<script lang="ts">
  import { untrack } from 'svelte';
  import { fleet, key } from '../lib/fleet.svelte';
  import { ui, type Draft } from '../lib/ui.svelte';
  import type { AgentDetail, AgentSummary, ImageInput, ModelInfo } from '../lib/protocol';
  import { ArrowUp, Image, MousePointerClick, Paperclip, Square, X } from '../lib/icons';
  import { slashMenu } from '../lib/slash';
  import UsageMeter from './UsageMeter.svelte';
  import { imageFiles, onImagePaste, readImage, type PickedImage } from '../lib/images';
  import ModelPicker from '../components/ModelPicker.svelte';
  import SlashMenu from './SlashMenu.svelte';

  let { agent: a, detail }: { agent: AgentSummary; detail?: AgentDetail } = $props();

  const THINKING = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh'];

  const k = $derived(key(a.hostId, a.id));
  // The view is keyed per agent, so the draft is fixed for this component's lifetime.
  const draft: Draft = untrack(() => ui.draft(key(a.hostId, a.id)));
  const running = $derived(a.status === 'running' || a.status === 'starting' || a.status === 'waiting');
  const readonly = $derived(a.status === 'stopped');
  const queued = $derived([...(detail?.queuedSteering ?? []).map((t) => ({ t, kind: 'Steer' })), ...(detail?.queuedFollowUp ?? []).map((t) => ({ t, kind: 'Queued' }))]);

  let mode = $state<'steer' | 'followUp'>('steer');
  let busy = $state(false);
  let dragOver = $state(false);
  let models = $state<ModelInfo[]>([]);
  let ta = $state<HTMLTextAreaElement>();
  let fileInput = $state<HTMLInputElement>();

  $effect(() => {
    fleet
      .loadModels(a.hostId)
      .then((m) => (models = m))
      .catch(() => {});
  });

  // ─── Slash commands: omp's catalog for this agent, loaded when the first `/` is typed ──────
  const commands = $derived(fleet.commands[k]);
  let commandsLoading = $state(false);
  /** Draft text the menu was dismissed (Esc) for; it reopens once the text changes. */
  let dismissedFor = $state<string | null>(null);
  let slashActive = $state(0);
  const typingSlash = $derived(draft.text.startsWith('/') && !draft.text.includes('\n'));
  const menu = $derived(typingSlash && dismissedFor !== draft.text ? slashMenu(draft.text, commands ?? []) : undefined);
  const menuOpen = $derived(!!menu && (menu.options.length > 0 || commandsLoading || menu.stage === 'command'));

  $effect(() => {
    if (!typingSlash || commands || commandsLoading) return;
    commandsLoading = true;
    fleet
      .loadCommands(a.hostId, a.id)
      .catch((err) => ui.error('Could not load slash commands', err))
      .finally(() => (commandsLoading = false));
  });
  // A new query starts at the best match.
  $effect(() => {
    void draft.text;
    slashActive = 0;
  });

  function pick(index: number) {
    const option = menu?.options[index];
    if (!option) return;
    draft.text = option.insert;
    ta?.focus();
  }

  // Auto-grow the textarea up to ~40% of the viewport.
  $effect(() => {
    void draft.text;
    if (!ta) return;
    ta.style.height = 'auto';
    ta.style.height = `${Math.min(ta.scrollHeight, window.innerHeight * 0.4)}px`;
  });

  // Focus the composer when the agent opens (desktop only: avoid popping the phone keyboard).
  $effect(() => {
    if (ta && matchMedia('(pointer: fine)').matches) ta.focus({ preventScroll: true });
  });

  function buildMessage(): { message: string; images: ImageInput[] } {
    const parts = [draft.text.trim()];
    const images: ImageInput[] = draft.images.map((i) => i.image);
    for (const el of draft.elements) {
      const r = el.inspect;
      const styles = Object.entries(r.styles)
        .map(([p, v]) => `  ${p}: ${v};`)
        .join('\n');
      parts.push(
        [
          `<selected-element url="${el.url}" selector="${r.selector}">`,
          `Box: ${Math.round(r.box.x)},${Math.round(r.box.y)} ${Math.round(r.box.width)}×${Math.round(r.box.height)}`,
          r.text ? `Text: ${r.text}` : '',
          `Computed styles:\n${styles}`,
          `HTML:\n${r.html}`,
          '</selected-element>',
        ]
          .filter(Boolean)
          .join('\n'),
      );
      if (r.screenshot) images.push({ mimeType: 'image/png', data: r.screenshot });
    }
    return { message: parts.filter(Boolean).join('\n\n'), images };
  }

  const canSend = $derived(!readonly && !busy && (!!draft.text.trim() || draft.images.length > 0 || draft.elements.length > 0));

  async function send() {
    if (!canSend) return;
    const { message, images } = buildMessage();
    busy = true;
    try {
      await fleet.call(
        'agents.prompt',
        { id: a.id, message, mode: running ? mode : 'auto', images: images.length ? images : undefined },
        a.hostId,
        60_000,
      );
      draft.text = '';
      draft.images = [];
      draft.elements = [];
    } catch (err) {
      ui.error('Could not send', err);
    } finally {
      busy = false;
    }
  }

  async function abort() {
    try {
      await fleet.call('agents.abort', { id: a.id }, a.hostId);
    } catch (err) {
      ui.error('Abort failed', err);
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (menuOpen && menu && !e.isComposing) {
      const n = menu.options.length;
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        if (n) slashActive = (slashActive + (e.key === 'ArrowDown' ? 1 : n - 1)) % n;
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        dismissedFor = draft.text;
        return;
      }
      const option = menu.options[slashActive];
      if (e.key === 'Tab' && option) {
        e.preventDefault();
        pick(slashActive);
        return;
      }
      // Enter completes; it runs the command once nothing is left to type.
      if (e.key === 'Enter' && !e.shiftKey && option && !(option.complete && option.insert === draft.text.trimEnd())) {
        e.preventDefault();
        if (option.complete) {
          draft.text = option.insert;
          void send();
        } else pick(slashActive);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    } else if (e.key === 'Escape' && running) {
      e.preventDefault();
      void abort();
    }
  }

  const attach = (im: PickedImage) => ui.attachImage(k, im.name, im.image);
  const readFile = (file: File) => void readImage(file).then((im) => im && attach(im));
  const onpaste = (e: ClipboardEvent) => onImagePaste(e, attach);

  function ondrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    imageFiles(e.dataTransfer?.files).forEach(readFile);
  }

  async function setModel(model: string) {
    try {
      fleet.agents[k] = await fleet.call('agents.setModel', { id: a.id, model }, a.hostId);
    } catch (err) {
      ui.error('Could not switch model', err);
    }
  }

  async function setThinking(level: string) {
    try {
      fleet.agents[k] = await fleet.call('agents.setThinking', { id: a.id, level }, a.hostId);
    } catch (err) {
      ui.error('Could not change thinking level', err);
    }
  }

  // Esc outside the composer aborts only when nothing else has focus (no button, field, menu,
  // drawer or dialog could be the intended target of the key).
  function onWindowKey(e: KeyboardEvent) {
    if (e.key !== 'Escape' || !running || ui.dialog || ui.paletteOpen || ui.drawerOpen || e.defaultPrevented) return;
    if (e.target === document.body) void abort();
  }
</script>

<svelte:window onkeydown={onWindowKey} />

{#if readonly}
  <div class="ro faint">This agent is archived and read-only.</div>
{:else}
  {#if queued.length}
    <div class="queued">
      {#each queued as q, i (i)}
        <div class="qrow"><span class="qk">{q.kind}</span><span class="ellipsis">{q.t}</span></div>
      {/each}
    </div>
  {/if}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="composer"
    class:drag={dragOver}
    ondragover={(e) => {
      e.preventDefault();
      dragOver = true;
    }}
    ondragleave={() => (dragOver = false)}
    {ondrop}
  >
    {#if draft.images.length || draft.elements.length}
      <div class="attach">
        {#each draft.elements as el (el.id)}
          <div class="chip-el" title={el.inspect.selector}>
            {#if el.inspect.screenshot}<img src="data:image/png;base64,{el.inspect.screenshot}" alt="" />{:else}<MousePointerClick size={13} />{/if}
            <span class="mono ellipsis">&lt;{el.inspect.tag}&gt; {el.inspect.selector}</span>
            <button aria-label="Remove element" onclick={() => (draft.elements = draft.elements.filter((x) => x.id !== el.id))}><X size={12} /></button>
          </div>
        {/each}
        {#each draft.images as im (im.id)}
          <div class="chip-el">
            <img src="data:{im.image.mimeType};base64,{im.image.data}" alt="" />
            <span class="ellipsis">{im.name}</span>
            <button aria-label="Remove image" onclick={() => (draft.images = draft.images.filter((x) => x.id !== im.id))}><X size={12} /></button>
          </div>
        {/each}
      </div>
    {/if}
    {#if menuOpen}<SlashMenu {menu} active={slashActive} loading={commandsLoading} onpick={pick} />{/if}
    <textarea
      bind:this={ta}
      bind:value={draft.text}
      rows="1"
      placeholder={running ? (mode === 'steer' ? 'Steer the agent mid-turn…' : 'Queue a follow-up for after this turn…') : 'Message the agent…  (/ for commands, Enter to send, Shift+Enter for a newline)'}
      role="combobox"
      aria-expanded={menuOpen}
      aria-controls={menuOpen ? 'slash-menu' : undefined}
      aria-activedescendant={menuOpen && menu?.options.length ? `slash-opt-${slashActive}` : undefined}
      aria-autocomplete="list"
      {onkeydown}
      {onpaste}
    ></textarea>
    <div class="bar">
      <button class="btn ghost icon sm" title="Attach image" aria-label="Attach image" onclick={() => fileInput?.click()}><Paperclip size={14} /></button>
      <input type="file" accept="image/*" multiple hidden bind:this={fileInput} onchange={(e) => [...(e.currentTarget.files ?? [])].forEach(readFile)} />
      <ModelPicker {models} value={a.model ?? ''} onselect={setModel} loading={!models.length} />
      <select class="mini think" value={a.thinking ?? ''} onchange={(e) => setThinking(e.currentTarget.value)} title="Thinking level" aria-label="Thinking level">
        {#if !a.thinking}<option value="">Thinking: default</option>{/if}
        {#each THINKING as t (t)}<option value={t}>Thinking: {t}</option>{/each}
      </select>
      <UsageMeter host={a.hostId} model={a.model} {running} />
      <span class="grow"></span>
      {#if running}
        <div class="seg" role="radiogroup" aria-label="Delivery">
          <button role="radio" aria-checked={mode === 'steer'} class:on={mode === 'steer'} onclick={() => (mode = 'steer')} title="Interrupt after the current tool call">Steer</button>
          <button role="radio" aria-checked={mode === 'followUp'} class:on={mode === 'followUp'} onclick={() => (mode = 'followUp')} title="Deliver when the turn ends">Queue</button>
        </div>
        <button class="btn sm stop" onclick={abort} title="Abort (Esc)" aria-label="Stop"><Square size={11} fill="currentColor" /><span class="stop-l">Stop</span></button>
      {/if}
      <button class="btn primary icon sm send" disabled={!canSend} onclick={send} title="Send (Enter)" aria-label="Send">
        {#if busy}<span class="spinner"></span>{:else}<ArrowUp size={15} />{/if}
      </button>
    </div>
    {#if dragOver}<div class="dropzone"><Image size={18} /> Drop images to attach</div>{/if}
  </div>
{/if}

<style>
  .ro {
    padding: 12px;
    text-align: center;
    border: 1px dashed var(--border-strong);
    border-radius: var(--r-lg);
    font-size: var(--fs-sm);
  }
  .queued {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    background: var(--bg-1);
    font-size: var(--fs-sm);
  }
  .qrow {
    display: flex;
    gap: 8px;
    min-width: 0;
    color: var(--text-2);
  }
  .qk {
    flex: none;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 19px;
  }
  .composer {
    position: relative;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border-strong);
    border-radius: 10px;
    background: var(--bg-1);
    transition:
      border-color var(--t-fast) var(--ease),
      box-shadow var(--t-fast) var(--ease);
  }
  .composer:focus-within {
    border-color: color-mix(in srgb, var(--accent) 60%, var(--border-strong));
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .composer.drag {
    border-color: var(--accent);
  }
  textarea {
    display: block;
    width: 100%;
    min-height: 44px;
    padding: 11px 12px 4px;
    border: 0;
    outline: none;
    background: transparent;
    resize: none;
    line-height: 1.5;
  }
  textarea::placeholder {
    color: var(--text-3);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px 6px;
  }
  .grow {
    flex: 1;
  }
  .mini {
    max-width: 170px;
    height: 24px;
    padding: 0 6px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .mini:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .seg {
    display: flex;
    padding: 2px;
    border-radius: var(--r);
    background: var(--bg-3);
  }
  .seg button {
    height: 20px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-weight: 500;
    cursor: pointer;
  }
  .seg button.on {
    background: var(--bg);
    color: var(--text);
    box-shadow: 0 0 0 1px var(--border-strong);
  }
  .stop {
    color: var(--red);
  }
  .send {
    width: 28px;
    height: 28px;
    border-radius: 8px;
  }
  .send .spinner {
    border-color: rgba(255, 255, 255, 0.35);
    border-top-color: #fff;
  }
  .attach {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 8px 8px 0;
  }
  .chip-el {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 280px;
    height: 30px;
    padding: 0 4px 0 3px;
    border: 1px solid var(--border-strong);
    border-radius: var(--r);
    background: var(--bg-2);
    font-size: var(--fs-xs);
  }
  .chip-el img {
    width: 24px;
    height: 24px;
    object-fit: cover;
    border-radius: 3px;
    background: var(--bg-3);
  }
  .chip-el button {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    flex: none;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-3);
    cursor: pointer;
  }
  .chip-el button:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .dropzone {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border-radius: inherit;
    background: color-mix(in srgb, var(--accent) 10%, var(--bg-1));
    color: var(--accent-text);
    font-weight: 500;
    pointer-events: none;
  }
  @container (max-width: 520px) {
    .mini {
      max-width: 110px;
    }
    .think,
    .stop-l {
      display: none;
    }
  }
</style>
