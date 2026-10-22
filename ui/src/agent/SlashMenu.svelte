<script lang="ts">
  import type { SlashMenu } from '../lib/slash';

  // Completion list above the composer. Selection and keys are driven by the composer; this
  // only renders, scrolls the active row into view and reports clicks.
  let { menu, active, loading, onpick }: { menu: SlashMenu | undefined; active: number; loading: boolean; onpick: (index: number) => void } = $props();

  let list = $state<HTMLDivElement>();
  $effect(() => {
    void active;
    list?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
  });
</script>

<div class="slash" role="listbox" id="slash-menu" aria-label="Slash commands" bind:this={list}>
  {#if menu?.stage === 'argument' && menu.command}
    <div class="head mono">/{menu.command.name}{#if menu.command.hint}<span class="faint">{` ${menu.command.hint}`}</span>{/if}</div>
  {/if}
  {#if loading && !menu?.options.length}
    <div class="none faint">Loading commands…</div>
  {:else if menu && !menu.options.length}
    <div class="none faint">{menu.stage === 'command' ? 'No matching command' : 'No matching option — Enter sends as typed'}</div>
  {/if}
  {#each menu?.options ?? [] as o, i (o.insert)}
    <!-- Mousedown keeps the textarea focused (no keyboard flicker on phones). -->
    <button
      type="button"
      role="option"
      id="slash-opt-{i}"
      aria-selected={i === active}
      class:active={i === active}
      onmousedown={(e) => e.preventDefault()}
      onclick={() => onpick(i)}
    >
      <span class="line">
        <span class="name mono">{o.label}</span>
        {#if o.hint}<span class="hint mono">{o.hint}</span>{/if}
        {#if o.badge}<span class="badge">{o.badge}</span>{/if}
      </span>
      {#if o.description}<span class="desc">{o.description}</span>{/if}
    </button>
  {/each}
  <div class="keys faint">↑↓ choose · Tab complete · Enter {menu?.options[active]?.complete ? 'run' : 'complete'} · Esc close</div>
</div>

<style>
  .slash {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 6px);
    z-index: 30;
    max-height: min(320px, 45vh);
    overflow-y: auto;
    padding: 4px;
    border-radius: var(--r-lg);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
  }
  .head {
    padding: 4px 8px 6px;
    color: var(--text);
    font-size: var(--fs-xs);
  }
  button {
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: 100%;
    padding: 5px 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  button.active {
    background: var(--bg-hover);
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .line {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .name {
    flex: none;
    font-size: 12px;
    font-weight: 500;
  }
  .hint {
    min-width: 0;
    overflow: hidden;
    color: var(--text-3);
    font-size: 11px;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .badge {
    flex: none;
    margin-left: auto;
    padding: 0 5px;
    border-radius: 3px;
    background: var(--bg-3);
    color: var(--text-2);
    font-size: 10px;
  }
  .desc {
    overflow: hidden;
    color: var(--text-2);
    font-size: var(--fs-xs);
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .none {
    padding: 6px 8px;
    font-size: var(--fs-xs);
  }
  .keys {
    position: sticky;
    bottom: -4px;
    margin: 2px -4px -4px;
    padding: 4px 12px;
    border-top: 1px solid var(--border);
    background: var(--bg-1);
    font-size: 10.5px;
  }
  @media (pointer: coarse) {
    .keys {
      display: none;
    }
    button {
      padding: 8px;
    }
  }
</style>
