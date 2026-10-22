// Slash-command completion for the composer: what the menu offers for the current draft and
// what accepting an entry turns the draft into.

import type { SlashCommand } from './protocol';

export interface SlashOption {
  /** Text the draft becomes when this option is accepted. */
  insert: string;
  /** Shown name: `/fast`, or a subcommand `on`. */
  label: string;
  hint?: string;
  description?: string;
  /** Badge for commands that are not omp builtins (`skill`, `custom`, …). */
  badge?: string;
  /** Accepting it leaves nothing to type: Enter may send right away. */
  complete: boolean;
}

export interface SlashMenu {
  /** Typing the command name, or its first argument. */
  stage: 'command' | 'argument';
  /** The command whose arguments are offered (`argument` stage). */
  command?: SlashCommand;
  options: SlashOption[];
}

const LIMIT = 60;
const BADGES: Record<string, string> = { skill: 'skill', extension: 'extension', custom: 'custom', mcp_prompt: 'mcp', file: 'file' };

/** Rank of `query` against `name`: prefix, then word-start, then substring, then subsequence. */
function score(name: string, query: string): number | undefined {
  if (!query) return 0;
  const n = name.toLowerCase();
  if (n.startsWith(query)) return 0;
  if (n.split(/[-_:.\s]/).some((w) => w.startsWith(query))) return 1;
  if (n.includes(query)) return 2;
  let i = 0;
  for (const c of n) if (c === query[i]) i++;
  return i === query.length ? 3 : undefined;
}

/** The menu for `text` (the whole draft), or `undefined` when it is not a slash command. */
export function slashMenu(text: string, commands: SlashCommand[]): SlashMenu | undefined {
  const name = /^\/(\S*)$/.exec(text);
  if (name) {
    const query = name[1].toLowerCase();
    const ranked: { c: SlashCommand; s: number }[] = [];
    for (const c of commands) {
      const s = Math.min(score(c.name, query) ?? 9, ...(c.aliases ?? []).map((a) => (score(a, query) ?? 9) + 0.5));
      // Descriptions only match a substring, and rank after every name match.
      const d = query.length > 2 && c.description?.toLowerCase().includes(query) ? 5 : 9;
      if (Math.min(s, d) < 9) ranked.push({ c, s: Math.min(s, d) });
    }
    // Bare `/` lists everything alphabetically; a query ranks by match quality, then brevity.
    ranked.sort((x, y) => (query ? x.s - y.s || x.c.name.length - y.c.name.length : 0) || x.c.name.localeCompare(y.c.name));
    return {
      stage: 'command',
      options: ranked.slice(0, LIMIT).map(({ c }) => {
        const takesArgs = !!c.hint || !!c.subcommands?.length;
        return {
          insert: takesArgs ? `/${c.name} ` : `/${c.name}`,
          label: `/${c.name}`,
          hint: c.hint,
          description: c.description,
          badge: BADGES[c.source],
          complete: !takesArgs,
        };
      }),
    };
  }
  const arg = /^\/(\S+) (\S*)$/.exec(text);
  if (!arg) return undefined;
  const command = commands.find((c) => c.name === arg[1] || c.aliases?.includes(arg[1]));
  if (!command?.subcommands?.length) return undefined;
  const query = arg[2].toLowerCase();
  const options = command.subcommands
    .map((s) => ({ s, rank: score(s.name, query) }))
    .filter((x) => x.rank !== undefined)
    .sort((x, y) => x.rank! - y.rank!)
    .map(({ s }) => {
      // A usage beyond the bare name (`kill <id>`) means more to type.
      const more = !!s.usage && s.usage.trim() !== s.name;
      return {
        insert: `/${arg[1]} ${s.name}${more ? ' ' : ''}`,
        label: s.name,
        hint: more ? s.usage : undefined,
        description: s.description,
        complete: !more,
      };
    });
  return { stage: 'argument', command, options };
}
