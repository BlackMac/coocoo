<script lang="ts">
  import { untrack } from 'svelte';
  import { fleet, key } from '../lib/fleet.svelte';
  import { RpcError } from '../lib/client';
  import { files, loadAgentImage, loadCardFile } from '../lib/files.svelte';
  import { ui } from '../lib/ui.svelte';
  import { router } from '../lib/router.svelte';
  import { ago, bytes } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import type { AgentSummary } from '../lib/protocol';
  import { ChevronLeft, ChevronRight, Maximize, MessageSquareText, ZoomIn, ZoomOut } from '../lib/icons';
  import Dialog from '../components/Dialog.svelte';

  // Everything visual of an agent, oldest to newest: screenshots and images its tool calls
  // returned, images sent with prompts, the mocks of its refinements and the context images of
  // the card it works on. Shows the newest unless another one was picked; `ui.imageFocus` (a
  // picture the agent just produced, or one clicked in the chat) selects one.
  let { agent }: { agent: AgentSummary } = $props();

  interface Visual {
    key: string;
    ts: number;
    title: string;
    source: string;
    mime: string;
    size: number;
    width?: number;
    height?: number;
    /** Starts loading it; returns its key into `files`. */
    load: () => string;
    /** A timeline item's time, for "Show in chat". */
    at?: number;
  }

  const host = $derived(agent.hostId);
  const k = $derived(key(agent.hostId, agent.id));
  const projectId = $derived(agent.projectId ?? undefined);
  const board = $derived(projectId ? fleet.boards[key(host, projectId)] : undefined);
  const items = $derived(fleet.images[k]);

  let error = $state<string | null>(null);
  /** The picked visual; `null` follows the newest. */
  let picked = $state<string | null>(null);
  /** Thumbnails scrolled into view (only those load). */
  let seen = $state<Record<string, true>>({});
  // Per agent (`k`, not the summary object, which changes on every update).
  $effect(() => {
    void k;
    untrack(() => {
      error = null;
      picked = null;
      seen = {};
      fleet.loadImages(agent.hostId, agent.id).catch((err) => {
        // A host on an older Coocoo has no `agents.images` (and stores no tool images).
        const old = err instanceof RpcError && err.code === 'unknown_method';
        const name = fleet.hosts[agent.hostId]?.name ?? 'This host';
        error = old ? `${name} runs an older Coocoo version. Update it to see the agent's screenshots here.` : err instanceof Error ? err.message : String(err);
      });
    });
  });
  $effect(() => {
    if (projectId && !board && fleet.conn === 'open') fleet.loadBoard(host, projectId).catch(() => {});
  });

  const visuals = $derived.by(() => {
    const out: Visual[] = [];
    const id = agent.id;
    for (const item of items ?? []) {
      const tool = item.tool;
      for (const img of item.images ?? []) {
        out.push({
          key: `t:${img.name}`,
          ts: item.ts,
          title: tool ? tool.intent || tool.name : 'Sent with a prompt',
          source: tool ? tool.name : 'Prompt',
          mime: img.mime,
          size: img.size,
          width: img.width,
          height: img.height,
          load: () => loadAgentImage(host, id, img),
          at: item.ts,
        });
      }
    }
    const pid = projectId;
    for (const card of pid ? (board?.cards ?? []) : []) {
      const r = card.refinement;
      if (r?.agentId === id) {
        for (const a of r.attachments ?? []) {
          const load = () => loadCardFile(host, pid!, card.id, 'attachment', a, r.at);
          out.push({ key: `r:${card.id}:${r.at}:${a.name}`, ts: r.at, title: a.name, source: `Refinement · ${card.title}`, mime: a.mime, size: a.size, load });
        }
      }
      if (card.agentId === id) {
        for (const a of card.images ?? []) {
          // Same version as the card editor, so both share the loaded file.
          const load = () => loadCardFile(host, pid!, card.id, 'image', a, a.size);
          out.push({ key: `c:${card.id}:${a.name}`, ts: card.createdAt, title: a.name, source: `Card · ${card.title}`, mime: a.mime, size: a.size, load });
        }
      }
    }
    return out.sort((a, b) => a.ts - b.ts);
  });

  const index = $derived.by(() => {
    const i = picked ? visuals.findIndex((v) => v.key === picked) : -1;
    return i >= 0 ? i : visuals.length - 1;
  });
  const current = $derived(visuals[index] as Visual | undefined);
  const src = $derived(current ? files[current.load()] : undefined);
  const isHtml = (v: Visual) => v.mime === 'text/html';

  // A requested picture is taken once, also when it arrived before this panel was shown.
  $effect(() => {
    const f = ui.imageFocus;
    if (!f || f.agent !== k) return;
    picked = f.name ? `t:${f.name}` : null;
    ui.imageFocus = null;
  });

  function pick(i: number) {
    picked = i === visuals.length - 1 ? null : visuals[i].key;
  }
  const step = (d: number) => visuals.length && pick((index + d + visuals.length) % visuals.length);

  let strip = $state<HTMLDivElement>();
  $effect(() => {
    strip?.children[index]?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });

  function lazy(node: HTMLElement, vkey: string) {
    const io = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        seen[vkey] = true;
        io.disconnect();
      },
      { root: strip, rootMargin: '0px 240px' },
    );
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  const when = (ts: number) => {
    const t = ago(ts, now.value);
    return t === 'now' ? 'just now' : `${t} ago`;
  };
  const meta = (v: Visual) => [v.source, when(v.ts), v.width && v.height ? `${v.width} × ${v.height}` : '', bytes(v.size)].filter(Boolean).join(' · ');

  function showInChat(v: Visual) {
    if (v.at) router.go({ name: 'agent', host, id: agent.id, at: v.at });
  }

  // The viewer takes the whole window; images fit it or show at their own size.
  let viewing = $state(false);
  let actual = $state(false);
  function open() {
    viewing = true;
    actual = false;
  }
  function onkeydown(e: KeyboardEvent) {
    if (!viewing || visuals.length < 2 || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return;
    e.preventDefault();
    step(e.key === 'ArrowLeft' ? -1 : 1);
  }
</script>

<svelte:window {onkeydown} />

<div class="images">
  {#if !items && !error}
    <div class="empty"><span class="spinner"></span></div>
  {:else if !visuals.length}
    <div class="empty">
      <strong>{error ? 'Images not available' : 'No images yet'}</strong>
      <span>{error ?? 'Screenshots the agent takes, images and design mocks it looks at or attaches, and the images of its card show up here.'}</span>
    </div>
  {:else if current}
    <div class="stage">
      {#if src === undefined}
        <span class="spinner"></span>
      {:else if !src}
        <span class="faint">Not available</span>
      {:else if isHtml(current)}
        <iframe srcdoc={src} sandbox="allow-scripts allow-forms" title={current.title}></iframe>
      {:else}
        <button class="shot" onclick={open} title="Open large"><img {src} alt={current.title} /></button>
      {/if}
    </div>
    <div class="cap">
      <div class="txt">
        <span class="title ellipsis" title={current.title}>{current.title}</span>
        <span class="faint ellipsis meta">{meta(current)}</span>
      </div>
      <span class="faint num pos">{index + 1}/{visuals.length}</span>
      <button class="btn ghost icon sm" disabled={visuals.length < 2} onclick={() => step(-1)} aria-label="Previous image" title="Previous"><ChevronLeft size={15} /></button>
      <button class="btn ghost icon sm" disabled={visuals.length < 2} onclick={() => step(1)} aria-label="Next image" title="Next"><ChevronRight size={15} /></button>
      {#if current.at}
        <button class="btn ghost icon sm" onclick={() => showInChat(current)} aria-label="Show in chat" title="Show in chat"><MessageSquareText size={14} /></button>
      {/if}
      <button class="btn ghost icon sm" disabled={!src} onclick={open} aria-label="Open large" title="Open large"><Maximize size={14} /></button>
    </div>
    <div class="strip" bind:this={strip}>
      {#each visuals as v, i (v.key)}
        {@const s = seen[v.key] ? files[v.load()] : undefined}
        <button class="th" class:on={i === index} onclick={() => pick(i)} title="{v.title} · {v.source}" use:lazy={v.key}>
          {#if !s}
            <span class="faint">{s === '' ? '?' : ''}</span>
          {:else if isHtml(v)}
            <iframe class="mini" srcdoc={s} sandbox="allow-scripts" title={v.title} tabindex="-1" inert></iframe>
          {:else}
            <img src={s} alt={v.title} />
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

{#if viewing && current}
  <Dialog title={visuals.length > 1 ? `${current.title} (${index + 1} of ${visuals.length})` : current.title} fill onclose={() => (viewing = false)}>
    {#snippet actions()}
      {#if visuals.length > 1}
        <button class="btn ghost icon sm" onclick={() => step(-1)} aria-label="Previous image" title="Previous (←)"><ChevronLeft size={15} /></button>
        <button class="btn ghost icon sm" onclick={() => step(1)} aria-label="Next image" title="Next (→)"><ChevronRight size={15} /></button>
      {/if}
      {#if !isHtml(current) && src}
        <button class="btn ghost icon sm" onclick={() => (actual = !actual)} aria-label={actual ? 'Fit to window' : 'Actual size'} title={actual ? 'Fit to window' : 'Actual size'}>
          {#if actual}<ZoomOut size={15} />{:else}<ZoomIn size={15} />{/if}
        </button>
      {/if}
    {/snippet}
    <div class="view" class:actual>
      {#if src === undefined}
        <span class="spinner"></span>
      {:else if !src}
        <span class="faint">Not available</span>
      {:else if isHtml(current)}
        <iframe srcdoc={src} sandbox="allow-scripts allow-forms" title={current.title}></iframe>
      {:else}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <img {src} alt={current.title} onclick={() => (actual = !actual)} />
      {/if}
    </div>
  </Dialog>
{/if}

<style>
  .images {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .images > .empty {
    flex: 1;
    padding: 24px;
    text-align: center;
  }
  .stage {
    flex: 1;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 12px;
    background: var(--bg-2);
  }
  .shot {
    display: flex;
    max-width: 100%;
    max-height: 100%;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--r);
    overflow: hidden;
    background: var(--bg);
    cursor: zoom-in;
  }
  .shot img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .stage iframe {
    width: 100%;
    height: 100%;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: #fff;
  }
  .cap {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 6px 8px 6px 12px;
    border-top: 1px solid var(--border);
    flex: none;
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .meta,
  .pos {
    font-size: var(--fs-xs);
  }
  .pos {
    padding: 0 6px;
  }
  .strip {
    display: flex;
    gap: 6px;
    padding: 8px 12px 10px;
    overflow-x: auto;
    border-top: 1px solid var(--border);
    flex: none;
  }
  .th {
    position: relative;
    display: grid;
    place-items: center;
    flex: none;
    width: 96px;
    height: 60px;
    padding: 0;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    background: var(--bg-2);
    cursor: pointer;
  }
  .th:hover {
    border-color: var(--border-strong);
  }
  .th.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .th img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: top;
  }
  /* The page laid out at 800 px wide, shown at an eighth of that. */
  .mini {
    position: absolute;
    top: 0;
    left: 0;
    width: 800px;
    height: 500px;
    border: 0;
    background: #fff;
    transform: scale(0.12);
    transform-origin: 0 0;
    pointer-events: none;
  }
  .view {
    flex: 1;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    border-radius: var(--r);
    background: var(--bg-2);
  }
  .view iframe {
    width: 100%;
    height: 100%;
    border: 0;
    background: #fff;
  }
  .view img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    cursor: zoom-in;
  }
  .view.actual {
    display: block;
    overflow: auto;
  }
  .view.actual img {
    display: block;
    max-width: none;
    max-height: none;
    margin: auto;
    cursor: zoom-out;
  }
</style>
