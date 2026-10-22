<script lang="ts">
  import type { AgentState } from '../lib/status';
  import { STATE_LABEL } from '../lib/status';

  let { state, size = 8 }: { state: AgentState; size?: number } = $props();
</script>

<span class="dot" data-state={state} style:--s="{size}px" title={STATE_LABEL[state]} role="img" aria-label={STATE_LABEL[state]}></span>

<style>
  .dot {
    position: relative;
    display: inline-block;
    flex: none;
    width: var(--s);
    height: var(--s);
    border-radius: 50%;
    background: var(--c);
    --c: var(--text-3);
  }
  [data-state='needs'] {
    --c: var(--amber);
  }
  [data-state='error'] {
    --c: var(--red);
  }
  [data-state='review'] {
    --c: var(--green);
  }
  [data-state='working'] {
    --c: var(--blue);
  }
  [data-state='working']::after {
    content: '';
    position: absolute;
    inset: -3px;
    border-radius: 50%;
    border: 1.5px solid var(--c);
    opacity: 0;
    animation: pulse 1.8s var(--ease) infinite;
  }
  [data-state='hibernated'],
  [data-state='archived'] {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px var(--text-3);
  }
  [data-state='archived'] {
    opacity: 0.55;
  }
  @keyframes pulse {
    0% {
      transform: scale(0.6);
      opacity: 0.7;
    }
    80%,
    100% {
      transform: scale(1.25);
      opacity: 0;
    }
  }
</style>
