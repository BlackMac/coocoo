<script lang="ts">
  import { fleet, key } from '../lib/fleet.svelte';
  import { ago } from '../lib/format';
  import { now } from '../lib/clock.svelte';
  import { ui } from '../lib/ui.svelte';
  import { href } from '../lib/router.svelte';
  import { agentState } from '../lib/status';
  import { IMPORT_INTERVALS, type AgentSummary, type BoardSource, type Card, type CardColumn } from '../lib/protocol';
  import { ArrowUp, ExternalLink, FileCode, Image, NotebookPen, GitCommitHorizontal, Inbox, Plus, RotateCw, Rss, ShieldCheck, ShieldOff, Terminal, Trash2, Webhook, X } from '../lib/icons';
  import StartCard from '../agent/StartCard.svelte';
  import RefinementView from '../agent/RefinementView.svelte';
  import CardFields, { type EditImage } from './CardFields.svelte';
  import StatusDot from '../components/StatusDot.svelte';

  // A project's Kanban board: inbox (from sources) → todo → doing → done. Cards move by drag and
  // drop or by the actions of an opened card; starting one hands it to an agent, which is then
  // shown on the card. `agent`: the agent view the board is shown in (Start goes to it).
  let { host, projectId, agent }: { host: string; projectId: string; agent?: AgentSummary } = $props();

  const COLUMNS: { id: CardColumn; label: string }[] = [
    { id: 'inbox', label: 'Inbox' },
    { id: 'todo', label: 'Todo' },
    { id: 'doing', label: 'Doing' },
    { id: 'done', label: 'Done' },
  ];

  const board = $derived(fleet.boards[key(host, projectId)]);
  const byColumn = $derived.by(() => {
    const out: Record<CardColumn, Card[]> = { inbox: [], todo: [], doing: [], done: [] };
    for (const c of board?.cards ?? []) out[c.column].push(c);
    return out;
  });

  let error = $state('');
  $effect(() => {
    if (fleet.conn !== 'open') return;
    fleet.loadBoard(host, projectId).then(
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
  /** Upload picked images one at a time (a single call with all of them could exceed the
   *  socket's message limit). `progress` shows "n/m" while it runs. */
  let progress = $state('');
  async function upload(cardId: string, images: EditImage[]) {
    const picked = images.flatMap((im) => ('picked' in im ? [im.picked] : []));
    for (const [i, p] of picked.entries()) {
      progress = `Uploading image ${i + 1}/${picked.length}…`;
      await fleet.call('boards.cardUpdate', { projectId, cardId, addImages: [{ name: p.name, ...p.image }] }, host, 120_000);
    }
  }

  const SUBMIT_KEY = /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘↵' : 'Ctrl+↵';

  // New card: an editor at the top of Todo.
  let creating = $state(false);
  let saving = $state(false);
  let draft = $state({ title: '', body: '', startUrl: '', doneUrl: '', images: [] as EditImage[] });
  const draftEmpty = $derived(!draft.title.trim() && !draft.body.trim() && !draft.images.length && !draft.startUrl && !draft.doneUrl);
  const resetDraft = () => (draft = { title: '', body: '', startUrl: '', doneUrl: '', images: [] });

  function startNew() {
    creating = true;
    openId = null;
  }
  function cancelNew() {
    if (draftEmpty) return void (creating = false);
    ui.dialog = {
      kind: 'confirm',
      title: 'Discard the new card?',
      message: 'What you typed and the images you added are lost.',
      action: 'Discard',
      danger: true,
      run: () => {
        resetDraft();
        creating = false;
      },
    };
  }
  async function addCard() {
    const title = draft.title.trim();
    if (!title) return ui.toast('error', 'The card needs a title');
    if (saving) return;
    saving = true;
    try {
      const card = await fleet.call('boards.cardAdd', { projectId, title, body: draft.body, startUrl: draft.startUrl, doneUrl: draft.doneUrl }, host);
      try {
        await upload(card.id, draft.images);
      } catch (err) {
        ui.error('The card was added, but an image could not be uploaded', err);
      }
      resetDraft();
      creating = false;
    } catch (err) {
      ui.error('Could not add the card', err);
    } finally {
      saving = false;
      progress = '';
    }
  }

  const move = (card: Card, column: CardColumn, index?: number) =>
    run('Could not move the card', () => fleet.call('boards.cardUpdate', { projectId, cardId: card.id, column, index }, host));
  const remove = (card: Card) => run('Could not delete the card', () => fleet.call('boards.cardDelete', { projectId, cardId: card.id }, host));

  // Opened card: the same editor in place.
  let openId = $state<string | null>(null);
  let edit = $state({ title: '', body: '', startUrl: '', doneUrl: '', images: [] as EditImage[] });
  function toggle(card: Card) {
    openId = openId === card.id ? null : card.id;
    edit = {
      title: card.title,
      body: card.body ?? '',
      startUrl: card.startUrl ?? '',
      doneUrl: card.doneUrl ?? '',
      images: (card.images ?? []).map((a) => ({ stored: a })),
    };
    // One editor at a time, unless the new card has content.
    if (openId && draftEmpty) creating = false;
  }
  async function save(card: Card) {
    if (saving) return;
    saving = true;
    try {
      const keep = new Set(edit.images.flatMap((im) => ('stored' in im ? [im.stored.name] : [])));
      const removeImages = (card.images ?? []).map((a) => a.name).filter((n) => !keep.has(n));
      await fleet.call(
        'boards.cardUpdate',
        { projectId, cardId: card.id, title: edit.title, body: edit.body, startUrl: edit.startUrl, doneUrl: edit.doneUrl, removeImages },
        host,
      );
      await upload(card.id, edit.images);
      openId = null;
    } catch (err) {
      ui.error('Could not save the card', err);
    } finally {
      saving = false;
      progress = '';
    }
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

  // ─── Importers (inbox sources) ─────────────────────────────────────────────────────────────
  let sourcesOpen = $state(false);
  let srcKind = $state<'feed' | 'command'>('feed');
  let srcTarget = $state('');
  let srcInterval = $state(60);
  let srcBusy = $state(false);
  async function addSource(e: SubmitEvent) {
    e.preventDefault();
    if (!srcTarget.trim()) return;
    srcBusy = true;
    const ok = await run('Could not add the importer', () =>
      fleet.call('boards.sourceAdd', { projectId, kind: srcKind, target: srcTarget.trim(), intervalMins: srcInterval }, host, 90_000),
    );
    srcBusy = false;
    if (ok) srcTarget = '';
  }
  let refreshing = $state<string | null>(null);
  async function refreshSource(id: string) {
    refreshing = id;
    await run('Could not run the importer', () => fleet.call('boards.sourceRefresh', { projectId, sourceId: id }, host, 90_000));
    refreshing = null;
  }
  const reschedule = (s: BoardSource, intervalMins: number) =>
    run('Could not change the schedule', () => fleet.call('boards.sourceUpdate', { projectId, sourceId: s.id, intervalMins }, host));
  function removeSource(s: BoardSource) {
    const remove = () => run('Could not delete the importer', () => fleet.call('boards.sourceRemove', { projectId, sourceId: s.id }, host));
    if (s.kind !== 'script') return void remove();
    // A script is work an agent did: ask first.
    ui.dialog = {
      kind: 'confirm',
      title: 'Delete importer',
      message: `Delete the script importer “${s.name}”? Cards it imported stay.`,
      action: 'Delete',
      danger: true,
      run: async () => {
        await remove();
      },
    };
  }
  let scriptOpen = $state<string | null>(null);
  const intervalLabel = (mins: number) =>
    ({ 15: 'every 15 min', 60: 'hourly', 360: 'every 6 h', 1440: 'daily' })[mins] ?? `every ${mins} min`;
  /** The preset schedules, plus a source's older custom one. */
  const intervalChoices = (mins: number): number[] =>
    (IMPORT_INTERVALS as readonly number[]).includes(mins) ? [...IMPORT_INTERVALS] : [...IMPORT_INTERVALS, mins].sort((a, b) => a - b);
  const sourceName = (id?: string) => {
    const s = board?.sources.find((x) => x.id === id);
    if (!s) return undefined;
    if (s.name) return s.name;
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
            {#if col.id === 'todo'}
              {#if creating}
                <div class="card open newcard">
                  <CardFields
                    {host}
                    {projectId}
                    bind:title={draft.title}
                    bind:body={draft.body}
                    bind:startUrl={draft.startUrl}
                    bind:doneUrl={draft.doneUrl}
                    bind:images={draft.images}
                    autofocus
                    onsubmit={addCard}
                    oncancel={cancelNew}
                  />
                  <div class="acts">
                    <button class="btn primary sm" disabled={saving || !draft.title.trim()} onclick={addCard}>{#if saving}<span class="spinner"></span>{/if}Add card</button>
                    <button class="btn sm" disabled={saving} onclick={cancelNew}>Cancel</button>
                    <span class="grow"></span>
                    <span class="faint kbd">{saving && progress ? progress : SUBMIT_KEY}</span>
                  </div>
                </div>
              {:else}
                <button class="newbtn" onclick={startNew}><Plus size={13} />New card</button>
              {/if}
            {/if}
            {#each cards as card, i (card.id)}
              {@const open = openId === card.id}
              {#if dropAt?.column === col.id && dropAt.index === i && dragId !== card.id}<div class="drop-line"></div>{/if}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="card"
                class:open
                class:ghost={dragId === card.id}
                draggable={!open}
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
                  {#if col.id === 'inbox'}
                    <button class="btn sm" title="Move to Todo" onclick={() => move(card, 'todo')}>Todo</button>
                  {:else if col.id === 'todo' && i === 0}
                    <StartCard {host} {projectId} {card} {agent} primary offerRefine />
                  {/if}
                </div>
                {#if !open && (card.agentId || card.source || card.commit || card.note || card.startUrl || card.doneUrl || card.refinement || card.refining || card.images?.length || card.addedBy)}
                  <div class="sub faint">
                    {#if card.refining || card.refinement}
                      <span class="refb" class:on={card.refining} title={card.refining ? 'An agent is refining it' : 'Has a refinement; open the card to read it'}>
                        <NotebookPen size={11} />{card.refining ? 'Refining…' : 'Refined'}
                      </span>
                    {/if}
                    {#if card.images?.length}<span class="imgc" title="{card.images.length} image{card.images.length === 1 ? '' : 's'}"><Image size={11} />{card.images.length}</span>{/if}
                    {#if card.agentId}{@render agentChip(card)}{/if}
                    {#if card.commit}<span class="mono"><GitCommitHorizontal size={11} />{card.commit.sha.slice(0, 7)}</span>{/if}
                    {#if card.startUrl || card.doneUrl}<span class="hooks" class:err={!!card.hookError} title={card.hookError ?? 'Webhooks on start/done'}><Webhook size={11} /></span>{/if}
                    {#if card.note}<span class="ellipsis">{card.note}</span>{:else if card.source}<span class="ellipsis">{sourceName(card.source)}</span>{:else if card.addedBy && !card.agentId}<span class="by">by {@render agentLink(card.addedBy.id, card.addedBy.title)}</span>{/if}
                  </div>
                {/if}
                {#if !open && card.hookError}<div class="herr">{card.hookError}</div>{/if}
                {#if open}
                  <div class="detail">
                    <CardFields
                      {host}
                      {projectId}
                      cardId={card.id}
                      bind:title={edit.title}
                      bind:body={edit.body}
                      bind:startUrl={edit.startUrl}
                      bind:doneUrl={edit.doneUrl}
                      bind:images={edit.images}
                      onsubmit={() => save(card)}
                      oncancel={() => (openId = null)}
                    />
                    {#if card.hookError}<div class="herr">{card.hookError}</div>{/if}
                    {#if card.agentId}<div class="faint">{col.id === 'done' ? 'Done by' : card.refining ? 'Being refined by' : 'Agent:'} {@render agentChip(card)}</div>{/if}
                    {#if card.refinement}<RefinementView {host} {projectId} {card} clamp />{/if}
                    {#if card.url}<a class="link" href={card.url} target="_blank" rel="noopener noreferrer"><ExternalLink size={11} />{card.url}</a>{/if}
                    {#if card.note}<div class="note"><span class="faint">Note:</span> {card.note}</div>{/if}
                    {#if card.commit}<div class="faint mono">{card.commit.sha.slice(0, 7)} {card.commit.summary}</div>{/if}
                    <div class="faint meta">Added {ago(card.createdAt, now.value)} ago{card.source ? ` from ${sourceName(card.source) ?? 'a removed source'}` : ''}{#if card.addedBy}&nbsp;by {@render agentLink(card.addedBy.id, card.addedBy.title)}{/if}{saving && progress ? ` · ${progress}` : ''}</div>
                    <div class="acts">
                      <button class="btn primary sm" disabled={saving} onclick={() => save(card)}>{#if saving}<span class="spinner"></span>{/if}Save</button>
                      {#if col.id === 'inbox' || col.id === 'todo'}
                        <StartCard {host} {projectId} {card} {agent} />
                        <StartCard {host} {projectId} {card} {agent} refine />
                      {/if}
                      {#if i > 0}<button class="btn ghost sm" title="Move to top" onclick={() => move(card, col.id, 0)}><ArrowUp size={12} />Top</button>{/if}
                      <select class="select mini" value="" aria-label="Move to column" onchange={(e) => { const c = e.currentTarget.value as CardColumn; if (c) void move(card, c); e.currentTarget.value = ''; }}>
                        <option value="">Move to…</option>
                        {#each COLUMNS.filter((c) => c.id !== col.id) as c (c.id)}<option value={c.id}>{c.label}</option>{/each}
                      </select>
                      <span class="grow"></span>
                      <button class="btn ghost icon sm danger" title="Delete card" aria-label="Delete card" onclick={() => remove(card)}><Trash2 size={13} /></button>
                    </div>
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
        <Rss size={12} />Importers<span class="count num">{board.sources.length}</span>
      </button>
      {#if sourcesOpen}
        {#each board.sources as s (s.id)}
          <div class="src">
            {#if s.kind === 'feed'}<Rss size={12} />{:else if s.kind === 'command'}<Terminal size={12} />{:else}<FileCode size={12} />{/if}
            <div class="sinfo">
              {#if s.kind === 'script'}
                <button class="sname" onclick={() => (scriptOpen = scriptOpen === s.id ? null : s.id)} aria-expanded={scriptOpen === s.id} title="Show the script">
                  <span class="ellipsis">{s.name}</span>
                </button>
              {:else}
                <span class="mono ellipsis" title={s.target}>{s.name ?? s.target}</span>
              {/if}
              <span class="faint smeta">
                {#if s.kind === 'script' && s.sandboxed !== undefined}
                  {#if s.sandboxed}
                    <span class="sbx" title="Can read the system and the project, write only its own directory"><ShieldCheck size={11} />sandboxed</span>
                  {:else}
                    <span class="sbx off" title="Sandboxing (Landlock) is not available on this host"><ShieldOff size={11} />not sandboxed</span>
                  {/if}
                {/if}
                {#if s.agentTitle}<span class="ellipsis">by {s.agentTitle}</span>{/if}
                {#if s.checkedAt}<span>ran {ago(s.checkedAt, now.value)} ago</span>{/if}
              </span>
              {#if s.error}<span class="serr">{s.error}</span>{/if}
              {#if scriptOpen === s.id && s.script}<pre class="script mono">{s.script}</pre>{/if}
            </div>
            <select class="select interval" value={s.intervalMins} onchange={(e) => reschedule(s, Number(e.currentTarget.value))} aria-label="Schedule">
              {#each intervalChoices(s.intervalMins) as mins (mins)}<option value={mins}>{intervalLabel(mins)}</option>{/each}
            </select>
            <button class="btn ghost icon sm" title="Run now" aria-label="Run now" disabled={refreshing === s.id} onclick={() => refreshSource(s.id)}>
              <span class:spin={refreshing === s.id}><RotateCw size={12} /></span>
            </button>
            <button class="btn ghost icon sm" title="Delete importer" aria-label="Delete importer" onclick={() => removeSource(s)}><X size={13} /></button>
          </div>
        {/each}
        <form class="sadd" onsubmit={addSource}>
          <input class="input mono target" bind:value={srcTarget} placeholder={srcKind === 'feed' ? 'https://example.com/feed.xml' : 'gh issue list --json title -q ".[].title"'} spellcheck="false" aria-label={srcKind === 'feed' ? 'Feed URL' : 'Command'} />
          <select class="select" bind:value={srcKind} aria-label="Importer kind">
            <option value="feed">RSS/Atom feed</option>
            <option value="command">Command</option>
          </select>
          <select class="select" bind:value={srcInterval} aria-label="Schedule">
            {#each IMPORT_INTERVALS as mins (mins)}<option value={mins}>{intervalLabel(mins)}</option>{/each}
          </select>
          <button class="btn sm" disabled={srcBusy || !srcTarget.trim()}>{#if srcBusy}<span class="spinner"></span>{:else}Add{/if}</button>
        </form>
        <p class="hint">
          {srcKind === 'feed'
            ? 'New feed items become inbox cards (the 20 newest when you add it).'
            : 'Runs in the project directory; each output line becomes an inbox card once.'}
          For anything else, ask an agent to build an importer, e.g. “put every new entry of X's changelog in the inbox”.
        </p>
      {/if}
    </section>
  {/if}
</div>

<!-- The agent a card was handed to: live status and title, or the remembered title once deleted. -->
{#snippet agentChip(card: Card)}
  {@render agentLink(card.agentId!, card.agentTitle)}
{/snippet}

{#snippet agentLink(id: string, title?: string)}
  {@const live = fleet.agents[key(host, id)]}
  {#if live}
    <a class="agent" href={href({ name: 'agent', host, id: live.id })} title="Open agent"><StatusDot state={agentState(live)} size={6} />{live.title}</a>
  {:else}
    <span class="agent gone" title="Agent deleted">{title ?? 'Deleted agent'}</span>
  {/if}
{/snippet}

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
  .newbtn {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 9px;
    border: 1px dashed var(--border-strong);
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .newbtn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .newcard {
    gap: 8px;
    padding: 8px;
    border-color: var(--accent);
  }
  .kbd {
    font-size: var(--fs-xs);
  }
  .acts .spinner {
    width: 10px;
    height: 10px;
  }
  .by {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    overflow: hidden;
  }
  .by .agent {
    max-width: none;
  }
  .imgc {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex: none;
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
  .agent {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex: none;
    max-width: 60%;
    overflow: hidden;
    color: var(--text-2);
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  a.agent:hover {
    color: var(--text);
    text-decoration: underline;
  }
  .agent.gone {
    font-style: italic;
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
  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .refb {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex: none;
    color: var(--accent);
  }
  .refb.on {
    animation: pulse 1.6s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.45;
    }
  }
  .hooks {
    display: inline-flex;
    flex: none;
  }
  .hooks.err {
    color: var(--red);
  }
  .herr {
    color: var(--red);
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
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .sname {
    display: flex;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .sname:hover {
    text-decoration: underline;
  }
  .smeta {
    display: flex;
    flex-wrap: wrap;
    gap: 0 8px;
    min-width: 0;
  }
  .sbx {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--green);
  }
  .sbx.off {
    color: var(--amber);
  }
  .script {
    max-height: 260px;
    margin: 4px 0 0;
    padding: 8px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    background: var(--bg);
    color: var(--text-2);
    font-size: var(--fs-xs);
    white-space: pre;
  }
  .src .interval {
    flex: none;
    width: auto;
    height: 24px;
    padding-left: 6px;
    font-size: var(--fs-xs);
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
  .spin {
    display: flex;
    animation: spin 0.8s linear infinite;
  }
</style>
