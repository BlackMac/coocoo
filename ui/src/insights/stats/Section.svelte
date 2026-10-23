<script lang="ts">
  // One card of the Stats tab: title with "?" (formula and source), a hint or controls on the
  // right, then the content.
  import type { Snippet } from 'svelte';
  import Help from '../Help.svelte';

  let {
    title,
    note,
    help,
    actions,
    children,
  }: {
    title: string;
    /** Faint hint on the right (what the section shows, how to interact). */
    note?: string;
    help?: Snippet;
    /** Controls on the right, instead of the note. */
    actions?: Snippet;
    children: Snippet;
  } = $props();
</script>

<section class="card sec" aria-label={title}>
  <h3>
    <span>{title}</span>
    {#if help}<Help label="How is “{title}” computed?">{@render help()}</Help>{/if}
    {#if actions}<span class="r">{@render actions()}</span>{:else if note}<span class="r faint">{note}</span>{/if}
  </h3>
  {@render children()}
</section>

<style>
  .sec {
    padding: 14px 16px;
    min-width: 0;
  }
  h3 {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px 8px;
    margin: 0 0 10px;
    font-size: var(--fs);
    font-weight: 600;
  }
  .r {
    margin-left: auto;
    font-size: var(--fs-sm);
    font-weight: 400;
    min-width: 0;
  }
  @media (max-width: 600px) {
    .sec {
      padding: 12px;
    }
    .r.faint {
      flex-basis: 100%;
      margin-left: 0;
      font-size: var(--fs-xs);
    }
  }
</style>
