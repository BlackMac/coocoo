// UI labels of the workflow-mining step ids (see `WorkflowSequence.steps`); the ids and their
// meaning are defined by the daemon's sequence miner.

const LABELS: Record<string, string> = {
  'agent.new': 'New agent',
  'agent.spawn': 'Spawned by agent',
  prompt: 'Prompt',
  steer: 'Steer',
  abort: 'Abort',
  'turn.error': 'Turn failed',
  'needs.you': 'Needs you',
  answer: 'Answer',
  'card.add': 'Card',
  'card.suggest': 'Suggested card',
  'card.accept': 'Accept card',
  'card.refine': 'Refine',
  'card.start': 'Start',
  'card.done': 'Done',
  commit: 'Commit',
  palette: 'Command palette',
  'model.change': 'Change model',
  leave: 'Leave app',
  'dialog.cancel': 'Cancel dialog',
  crash: 'Crash',
  compaction: 'Compaction',
  'api.limit': 'API limit',
};

/** Steps that count a sequence occurrence as friction; highlighted in the sequence. */
export const FRICTION_STEPS: Record<string, true> = { abort: true, 'turn.error': true, crash: true, 'api.limit': true, 'dialog.cancel': true };

export function stepLabel(id: string): string {
  const label = LABELS[id];
  if (label) return label;
  if (id.startsWith('panel.')) {
    const panel = id.slice('panel.'.length);
    return `${panel.charAt(0).toUpperCase()}${panel.slice(1)} panel`;
  }
  return id;
}
