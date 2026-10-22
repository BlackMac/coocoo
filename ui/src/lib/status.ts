// Maps raw agent status to the attention-oriented display states used across the UI.

import type { AgentSummary } from './protocol';

/** Display state, in triage order. */
export type AgentState = 'needs' | 'error' | 'working' | 'review' | 'idle' | 'hibernated' | 'archived';

export function agentState(a: AgentSummary): AgentState {
  if (a.status === 'waiting' || a.pendingRequests > 0) return 'needs';
  if (a.status === 'error') return 'error';
  if (a.status === 'running' || a.status === 'starting') return 'working';
  if (a.status === 'stopped') return 'archived';
  if (a.unread) return 'review';
  return a.status === 'hibernated' ? 'hibernated' : 'idle';
}

export const STATE_LABEL: Record<AgentState, string> = {
  needs: 'Needs you',
  error: 'Error',
  working: 'Working',
  review: 'Ready for review',
  idle: 'Idle',
  hibernated: 'Hibernated',
  archived: 'Archived',
};

export const STATE_RANK: Record<AgentState, number> = {
  needs: 0,
  error: 1,
  working: 2,
  review: 3,
  idle: 4,
  hibernated: 5,
  archived: 6,
};

/** Overview groups: errors sit with "Needs you", archived with "Hibernated". */
export const GROUPS: { id: string; label: string; states: AgentState[] }[] = [
  { id: 'needs', label: 'Needs you', states: ['needs', 'error'] },
  { id: 'working', label: 'Working', states: ['working'] },
  { id: 'review', label: 'Ready for review', states: ['review'] },
  { id: 'idle', label: 'Idle', states: ['idle'] },
  { id: 'hibernated', label: 'Hibernated & archived', states: ['hibernated', 'archived'] },
];
