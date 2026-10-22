<script lang="ts">
  let { online, size = 6 }: { online: boolean; size?: number } = $props();
</script>

<span class="hd" class:online style:--s="{size}px" title={online ? 'Online' : 'Offline'}></span>

<style>
  .hd {
    display: inline-block;
    flex: none;
    width: var(--s);
    height: var(--s);
    border-radius: 50%;
    box-shadow: inset 0 0 0 1.5px var(--text-3);
  }
  .online {
    background: var(--green);
    box-shadow: none;
  }
</style>
