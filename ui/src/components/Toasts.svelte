<script lang="ts">
  import { ui } from '../lib/ui.svelte';
  import { CircleAlert, CircleCheck, Info, X } from '../lib/icons';
</script>

<div class="toasts" aria-live="polite">
  {#each ui.toasts as t (t.id)}
    <div class="toast" data-kind={t.kind}>
      <span class="ic">
        {#if t.kind === 'error'}<CircleAlert size={15} />{:else if t.kind === 'success'}<CircleCheck size={15} />{:else}<Info size={15} />{/if}
      </span>
      <div class="txt">
        <strong>{t.title}</strong>
        {#if t.detail}<span>{t.detail}</span>{/if}
      </div>
      <button class="btn ghost icon sm" aria-label="Dismiss" onclick={() => ui.dismiss(t.id)}><X size={13} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 80;
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: min(380px, calc(100vw - 32px));
    pointer-events: none;
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 8px 10px 12px;
    border-radius: var(--r-lg);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    pointer-events: auto;
    animation: in var(--t) var(--ease);
  }
  .ic {
    margin-top: 1px;
    color: var(--text-2);
  }
  [data-kind='error'] .ic {
    color: var(--red);
  }
  [data-kind='success'] .ic {
    color: var(--green);
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .txt strong {
    font-weight: 500;
  }
  .txt span {
    color: var(--text-2);
    font-size: var(--fs-sm);
    overflow-wrap: anywhere;
  }
  @keyframes in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
</style>
