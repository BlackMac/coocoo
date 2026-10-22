<script lang="ts">
  let { value, onsave, label }: { value: string; onsave: (v: string) => void; label: string } = $props();

  let editing = $state(false);
  let draft = $state('');
  let input = $state<HTMLInputElement>();

  function start() {
    draft = value;
    editing = true;
    queueMicrotask(() => {
      input?.focus();
      input?.select();
    });
  }

  function commit() {
    if (!editing) return;
    editing = false;
    const v = draft.trim();
    if (v && v !== value) onsave(v);
  }
</script>

{#if editing}
  <input
    class="ie-input"
    bind:this={input}
    bind:value={draft}
    aria-label={label}
    onblur={commit}
    onkeydown={(e) => {
      if (e.key === 'Enter') commit();
      else if (e.key === 'Escape') {
        e.stopPropagation();
        editing = false;
      }
    }}
  />
{:else}
  <button class="ie" title="Click to rename" onclick={start}>{value}</button>
{/if}

<style>
  .ie,
  .ie-input {
    min-width: 0;
    max-width: 100%;
    /* Optical alignment of the text with content below; `left` keeps the layout box intact. */
    position: relative;
    left: -6px;
    margin: 0;
    padding: 1px 5px;
    border: 1px solid transparent;
    border-radius: var(--r);
    background: transparent;
    color: inherit;
    font: inherit;
    letter-spacing: inherit;
  }
  .ie {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
    cursor: text;
  }
  .ie:hover {
    border-color: var(--border);
  }
  .ie-input {
    border-color: var(--accent);
    outline: none;
    box-shadow: 0 0 0 3px var(--accent-soft);
    width: 100%;
  }
</style>
