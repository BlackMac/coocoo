<script lang="ts">
  import { fleet, key } from '../lib/fleet.svelte';
  import { ago } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import { ui } from '../lib/ui.svelte';
  import type { AgentSummary, Card, CardColumn } from '../lib/protocol';
  import { ArrowUp, ExternalLink, GitCommitHorizontal, Inbox, Play, Plus, RotateCw, Rss, Terminal, Trash2, X } from '../lib/icons';

  // An agent's Kanban board: inbox (from sources) → todo → doing → done. Cards move by drag and
  // drop or by the actions of an opened card; starting a card prompts the agent with it.
  let { agent: a }: { agent: AgentSummary } = $props();

  const COLUMNS: { id: CardColumn; label: string }[] = [
    { id: 'inbox', label: 'Inbox' },
    { id: 'todo', label: 'Todo' },
    { id: 'doing', label: 'Doing' },
    { id: 'done', label: 'Done' },
  ];

  const k = $derived(key(a.hostId, a.id));
  const board = $derived(fleet.boards[k]);
  const byColumn = $derived.by(() => {
    const out: Record<CardColumn, Card[]> = { inbox: [], todo: [], doing: [], done: [] };
    for (const c of board?.cards ?? []) out[c.column].push(c);
    return out;
  });
  const readonly = $derived(a.status === 'stopped');

  let error = $state('');
  $effect(() => {
    if (fleet.conn !== 'open') return;
    fleet.loadBoard(a.hostId, a.id).then(
      () => (error = ''),
      (err) => (error = err instanceof Error ? err.message : String(err)),
    );
  });

  async function run<T>(label: string, f: () => Promise<T>): Promise<T | undefined> {
    try {
      return await f();
    } catch (err) {
      ui.error(label, err);
      return undefined;
    }
  }

  // ─── Cards ─────────────────────────────────────────────────────────────────────────────────
  let newTitle = $state('');
  async function add(e: SubmitEvent) {
    e.preventDefault();
    const title = newTitle.trim();
    if (!title) return;
    newTitle = '';
    await run('Could not add the card', () => fleet.call('agents.cardAdd', { id: a.id, title }, a.hostId));
  }

  const move = (card: Card, column: CardColumn, index?: number) =>
    run('Could not move the card', () => fleet.call('agents.cardUpdate', { id: a.id, cardId: card.id, column, index }, a.hostId));
  const start = (card: Card) => run('Could not start the card', () => fleet.call('agents.cardStart', { id: a.id, cardId: card.id }, a.hostId));
  const remove = (card: Card) => run('Could not delete the card', () => fleet.call('agents.cardDelete', { id: a.id, cardId: card.id }, a.hostId));

  let openId = $state<string | null>(null);
  let editTitle = $state('');
  let editBody = $state('');
  function toggle(card: Card) {
    openId = openId === card.id ? null : card.id;
    editTitle = card.title;
    editBody = card.body ?? '';
  }
  async function save(card: Card) {
    const ok = await run('Could not save the card', () =>
      fleet.call('agents.cardUpdate', { id: a.id, cardId: card.id, title: editTitle, body: editBody }, a.hostId),
    );
    if (ok) openId = null;
  }

  // ─── Drag and drop (desktop) ───────────────────────────────────────────────────────────────
  let dragId = $state<string | null>(null);
  /** Drop target: column and index the card would land at. */
  let dropAt = $state<{ column: CardColumn; index: number } | null>(null);

  function overCard(e: DragEvent, column: CardColumn, index: number) {
    if (!dragId) return;
    e.preventDefault();
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    dropAt = { column, index: e.clientY > r.top + r.height / 2 ? index + 1 : index };
  }
  function overColumn(e: DragEvent, column: CardColumn) {
    if (!dragId) return;
    e.preventDefault();
    if (dropAt?.column !== column) dropAt = { column, index: byColumn[column].length };
  }
  function drop(e: DragEvent) {
    e.preventDefault();
    const card = board?.cards.find((c) => c.id === dragId);
    const target = dropAt;
    dragId = null;
    dropAt = null;
    if (!card || !target) return;
    // Indices count the dragged card while it is still in its column.
    let index = target.index;
    const from = byColumn[card.column].indexOf(card);
    if (card.column === target.column && from < index) index--;
    if (card.column === target.column && from === index) return;
    void move(card, target.column, index);
  }

  // ─── Sources ───────────────────────────────────────────────────────────────────────────────
  let sourcesOpen = $state(false);
  let srcKind = $state<'feed' | 'command'>('feed');
  let srcTarget = $state('');
  let srcInterval = $state(15);
  let srcBusy = $state(false);
  async function addSource(e: SubmitEvent) {
    e.preventDefault();
    if (!srcTarget.trim()) return;
    srcBusy = true;
    const ok = await run('Could not add the source', () =>
      fleet.call('agents.boardSourceAdd', { id: a.id, kind: srcKind, target: srcTarget.trim(), intervalMins: srcInterval }, a.hostId, 60_000),
    );
    srcBusy = false;
    if (ok) srcTarget = '';
  }
  let refreshing = $state<string | null>(null);
  async function refreshSource(id: string) {
    refreshing = id;
    await run('Could not refresh the source', () => fleet.call('agents.boardSourceRefresh', { id: a.id, sourceId: id }, a.hostId, 60_000));
    refreshing = null;
  }
  const removeSource = (id: string) => run('Could not remove the source', () => fleet.call('agents.boardSourceRemove', { id: a.id, sourceId: id }, a.hostId));
  const sourceName = (id?: string) => {
    const s = board?.sources.find((x) => x.id === id);
    if (!s) return undefined;
    return s.kind === 'feed' ? s.target.replace(/^https?:\/\/(www\.)?/, '').split('/')[0] : s.target;
  };
</script>

<div class="board">
  {#if error && !board}
    <div class="empty"><strong>Could not load the board</strong><span>{error}</span></div>
  {:else if !board}
    <div class="empty"><span class="spinner"></span></div>
  {:else}
    <div class="cols" class:dragging={!!dragId}>
      {#each COLUMNS as col (col.id)}
        {@const cards = byColumn[col.id]}
        {#if col.id !== 'inbox' || cards.length || board.sources.length}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <section class="col" data-col={col.id} ondragover={(e) => overColumn(e, col.id)} ondrop={drop} class:target={dropAt?.column === col.id}>
            <h3>
              {#if col.id === 'inbox'}<Inbox size={12} />{/if}
              {col.label}<span class="count num">{cards.length}</span>
            </h3>
            {#if col.id === 'todo' && !readonly}
              <form class="add" onsubmit={add}>
                <Plus size={13} />
                <input class="input" bind:value={newTitle} placeholder="Add a card…" />
              </form>
            {/if}
            {#each cards as card, i (card.id)}
              {@const open = openId === card.id}
              {#if dropAt?.column === col.id && dropAt.index === i && dragId !== card.id}<div class="drop-line"></div>{/if}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="card"
                class:open
                class:ghost={dragId === card.id}
                draggable={!readonly && !open}
                ondragstart={(e) => {
                  dragId = card.id;
                  e.dataTransfer?.setData('text/plain', card.title);
                }}
                ondragend={() => {
                  dragId = null;
                  dropAt = null;
                }}
                ondragover={(e) => overCard(e, col.id, i)}
                ondrop={drop}
              >
                <div class="ctop">
                  <button class="title" onclick={() => toggle(card)} aria-expanded={open}>{card.title}</button>
                  {#if !readonly}
                    {#if col.id === 'inbox'}
                      <button class="btn sm" title="Move to Todo" onclick={() => move(card, 'todo')}>Todo</button>
                    {:else if col.id === 'todo' && i === 0}
                      <button class="btn primary sm" title="Let the agent work on it" onclick={() => start(card)}><Play size={11} fill="currentColor" />Start</button>
                    {/if}
                  {/if}
                </div>
                {#if !open && (card.source || card.commit || card.note)}
                  <div class="sub faint">
                    {#if card.commit}<span class="mono"><GitCommitHorizontal size={11} />{card.commit.sha.slice(0, 7)}</span>{/if}
                    {#if card.note}<span class="ellipsis">{card.note}</span>{:else if card.source}<span class="ellipsis">{sourceName(card.source)}</span>{/if}
                  </div>
                {/if}
                {#if open}
                  <div class="detail">
                    {#if readonly}
                      {#if card.body}<p class="body">{card.body}</p>{/if}
                    {:else}
                      <input class="input" bind:value={editTitle} aria-label="Title" />
                      <textarea class="textarea" bind:value={editBody} rows="4" placeholder="Details for the agent (optional)"></textarea>
                    {/if}
                    {#if card.url}<a class="link" href={card.url} target="_blank" rel="noopener noreferrer"><ExternalLink size={11} />{card.url}</a>{/if}
                    {#if card.note}<div class="note"><span class="faint">Agent:</span> {card.note}</div>{/if}
                    {#if card.commit}<div class="faint mono">{card.commit.sha.slice(0, 7)} {card.commit.summary}</div>{/if}
                    <div class="faint meta">Added {ago(card.createdAt, now.value)} ago{card.source ? ` from ${sourceName(card.source) ?? 'a removed source'}` : ''}</div>
                    {#if !readonly}
                      <div class="acts">
                        <button class="btn primary sm" onclick={() => save(card)}>Save</button>
                        {#if col.id === 'inbox' || col.id === 'todo'}<button class="btn sm" onclick={() => start(card)}><Play size={11} fill="currentColor" />Start</button>{/if}
                        {#if i > 0}<button class="btn ghost sm" title="Move to top" onclick={() => move(card, col.id, 0)}><ArrowUp size={12} />Top</button>{/if}
                        <select class="select mini" value="" aria-label="Move to column" onchange={(e) => { const c = e.currentTarget.value as CardColumn; if (c) void move(card, c); e.currentTarget.value = ''; }}>
                          <option value="">Move to…</option>
                          {#each COLUMNS.filter((c) => c.id !== col.id) as c (c.id)}<option value={c.id}>{c.label}</option>{/each}
                        </select>
                        <span class="grow"></span>
                        <button class="btn ghost icon sm danger" title="Delete card" aria-label="Delete card" onclick={() => remove(card)}><Trash2 size={13} /></button>
                      </div>
                    {/if}
                  </div>
                {/if}
              </div>
            {/each}
            {#if dropAt?.column === col.id && dropAt.index >= cards.length}<div class="drop-line"></div>{/if}
            {#if !cards.length && col.id !== 'todo'}<div class="none faint">{col.id === 'inbox' ? 'New entries from your sources land here.' : 'Nothing here'}</div>{/if}
          </section>
        {/if}
      {/each}
    </div>

    <section class="sources">
      <button class="shead" onclick={() => (sourcesOpen = !sourcesOpen)} aria-expanded={sourcesOpen}>
        <Rss size={12} />Inbox sources<span class="count num">{board.sources.length}</span>
      </button>
      {#if sourcesOpen}
        {#each board.sources as s (s.id)}
          <div class="src">
            {#if s.kind === 'feed'}<Rss size={12} />{:else}<Terminal size={12} />{/if}
            <div class="sinfo">
              <span class="mono ellipsis" title={s.target}>{s.target}</span>
              <span class="faint">
                every {s.intervalMins} min{s.checkedAt ? ` · checked ${ago(s.checkedAt, now.value)} ago` : ''}
              </span>
              {#if s.error}<span class="serr">{s.error}</span>{/if}
            </div>
            {#if !readonly}
              <button class="btn ghost icon sm" title="Check now" aria-label="Check now" disabled={refreshing === s.id} onclick={() => refreshSource(s.id)}>
                <span class:spin={refreshing === s.id}><RotateCw size={12} /></span>
              </button>
              <button class="btn ghost icon sm" title="Remove source" aria-label="Remove source" onclick={() => removeSource(s.id)}><X size={13} /></button>
            {/if}
          </div>
        {/each}
        {#if !readonly}
          <form class="sadd" onsubmit={addSource}>
            <input class="input mono target" bind:value={srcTarget} placeholder={srcKind === 'feed' ? 'https://example.com/feed.xml' : 'gh issue list --json title -q ".[].title"'} spellcheck="false" aria-label={srcKind === 'feed' ? 'Feed URL' : 'Command'} />
            <select class="select" bind:value={srcKind} aria-label="Source kind">
              <option value="feed">RSS/Atom feed</option>
              <option value="command">Command</option>
            </select>
            <label class="every faint">every <input class="input num" type="number" min="5" bind:value={srcInterval} /> min</label>
            <button class="btn sm" disabled={srcBusy || !srcTarget.trim()}>{#if srcBusy}<span class="spinner"></span>{:else}Add{/if}</button>
          </form>
          <p class="hint">
            {srcKind === 'feed'
              ? 'New feed items become inbox cards (the 20 newest when you add it).'
              : "Runs in the agent's directory; each output line becomes an inbox card once."}
          </p>
        {/if}
      {/if}
    </section>
  {/if}
</div>

<style>
  .board {
    height: 100%;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    container-type: inline-size;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 40px 12px;
    color: var(--text-2);
    text-align: center;
  }
  .cols {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 14px;
    align-items: start;
  }
  @container (min-width: 760px) {
    .cols {
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    }
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    padding: 4px;
    margin: -4px;
    border-radius: var(--r-lg);
  }
  .dragging .col.target {
    background: var(--bg-hover);
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    color: var(--text-3);
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .count {
    color: var(--text-3);
    font-weight: 500;
  }
  .add {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-3);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 7px 8px 7px 10px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--bg-1);
  }
  .card[draggable='true'] {
    cursor: grab;
  }
  .card.ghost {
    opacity: 0.4;
  }
  [data-col='done'] .card:not(.open) .title {
    color: var(--text-2);
  }
  [data-col='doing'] .card {
    border-color: color-mix(in srgb, var(--blue) 45%, var(--border));
  }
  .ctop {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }
  .title {
    flex: 1;
    min-width: 0;
    padding: 2px 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-sm);
    text-align: left;
    overflow-wrap: anywhere;
    cursor: pointer;
  }
  .ctop .btn {
    flex: none;
    gap: 4px;
  }
  .sub {
    display: flex;
    gap: 8px;
    min-width: 0;
    font-size: var(--fs-xs);
  }
  .sub .mono {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex: none;
  }
  .detail {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 4px;
    font-size: var(--fs-sm);
  }
  .textarea {
    height: auto;
    padding: 6px 9px;
  }
  .body {
    margin: 0;
    white-space: pre-wrap;
    color: var(--text-2);
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .note {
    color: var(--text-2);
  }
  .meta {
    font-size: var(--fs-xs);
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .acts .btn {
    gap: 4px;
  }
  .mini {
    width: auto;
    height: 26px;
    font-size: var(--fs-xs);
  }
  .grow {
    flex: 1;
  }
  .danger {
    color: var(--red);
  }
  .drop-line {
    height: 2px;
    margin: -1px 0;
    border-radius: 2px;
    background: var(--accent);
  }
  .none {
    padding: 4px 2px;
    font-size: var(--fs-xs);
  }
  .sources {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .shead {
    display: flex;
    align-items: center;
    gap: 6px;
    align-self: flex-start;
    padding: 2px 0;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-weight: 600;
    cursor: pointer;
  }
  .src {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    color: var(--text-3);
    font-size: var(--fs-xs);
  }
  .src > :global(svg) {
    margin-top: 2px;
    flex: none;
  }
  .sinfo {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    min-width: 0;
    color: var(--text-2);
  }
  .serr {
    color: var(--red);
  }
  .sadd {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .sadd .select {
    width: auto;
  }
  .sadd > .target {
    flex: 1 1 100%;
  }
  .every {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
  }
  .every .input {
    width: 56px;
  }
  .spin {
    display: flex;
    animation: spin 0.8s linear infinite;
  }
</style>
