<script lang="ts">
  let { patch }: { patch: string } = $props();

  interface Line {
    t: 'hunk' | 'add' | 'del' | 'ctx' | 'meta';
    o?: number;
    n?: number;
    text: string;
  }

  const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/;

  const lines = $derived.by<Line[]>(() => {
    const out: Line[] = [];
    let o = 0;
    let n = 0;
    const raw = patch.split('\n');
    if (raw.at(-1) === '') raw.pop();
    for (const l of raw) {
      const c = l[0];
      if (c === '@') {
        const m = HUNK.exec(l);
        if (m) {
          o = Number(m[1]);
          n = Number(m[2]);
        }
        out.push({ t: 'hunk', text: l });
      } else if (c === '+') out.push({ t: 'add', n: n++, text: l.slice(1) });
      else if (c === '-') out.push({ t: 'del', o: o++, text: l.slice(1) });
      else if (c === '\\') out.push({ t: 'meta', text: l });
      else out.push({ t: 'ctx', o: o++, n: n++, text: l.slice(1) });
    }
    return out;
  });
</script>

<div class="diff mono">
  {#each lines as l, i (i)}
    <div class="dl" data-t={l.t}>
      {#if l.t === 'hunk' || l.t === 'meta'}
        <span class="hk">{l.text}</span>
      {:else}
        <span class="no">{l.o ?? ''}</span><span class="no">{l.n ?? ''}</span><span class="sg">{l.t === 'add' ? '+' : l.t === 'del' ? '−' : ''}</span><span class="tx">{l.text}</span>
      {/if}
    </div>
  {/each}
</div>

<style>
  .diff {
    font-size: 11.5px;
    line-height: 1.6;
    overflow-x: auto;
  }
  .dl {
    display: grid;
    grid-template-columns: 40px 40px 16px max-content;
    min-width: 100%;
  }
  .dl[data-t='hunk'],
  .dl[data-t='meta'] {
    display: block;
  }
  .hk {
    display: block;
    padding: 3px 12px;
    background: var(--accent-soft);
    color: var(--accent-text);
    white-space: pre;
  }
  .dl[data-t='meta'] .hk {
    background: transparent;
    color: var(--text-3);
  }
  .no {
    padding-right: 8px;
    color: var(--text-3);
    text-align: right;
    user-select: none;
    opacity: 0.8;
  }
  .sg {
    color: var(--text-3);
    text-align: center;
    user-select: none;
  }
  .tx {
    padding-right: 16px;
    white-space: pre;
    tab-size: 4;
  }
  [data-t='add'] {
    background: var(--add-bg);
  }
  [data-t='add'] .sg,
  [data-t='add'] .tx {
    color: var(--add-text);
  }
  [data-t='del'] {
    background: var(--del-bg);
  }
  [data-t='del'] .sg,
  [data-t='del'] .tx {
    color: var(--del-text);
  }
  [data-t='add'] .tx,
  [data-t='del'] .tx {
    color: var(--text);
  }
</style>
