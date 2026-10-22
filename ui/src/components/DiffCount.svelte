<script lang="ts">
  let { add, del, files }: { add: number; del: number; files?: number } = $props();
</script>

<span class="dc num">
  {#if files != null}<span class="faint">{files} file{files === 1 ? '' : 's'}</span>{/if}
  <span class="add">+{add}</span><span class="del">−{del}</span>
</span>

<style>
  .dc {
    display: inline-flex;
    gap: 5px;
    font-family: var(--mono);
    font-size: 11.5px;
    white-space: nowrap;
  }
</style>
