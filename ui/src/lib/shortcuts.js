// The Keyboard shortcuts sheet's data (design-notes §7, audit §7,
// 04-keyboard-shortcuts-*.png). General, Workspace and Dock come from the
// bindings themselves (`CHORDS`, `LEADER`), so the sheet cannot list a key
// that does nothing; the pane-local keys are static lists, as each pane's key
// handler reads them.

import { CHORDS, LEADER } from './keyboard.js';

/** The leader, shown once at the top of the sheet. */
export const LEADER_KEY = '⌘;';

/** Leader commands of the split layout (removed with the single pane, §4.1): never in the sheet. */
export const REMOVED = new Set(['pane.split.right', 'pane.split.below', 'pane.zoom', 'focus.left', 'focus.right']);

const GLYPH = { cmd: '⌘', shift: '⇧', alt: '⌥', ctrl: '⌃', enter: '⏎', escape: 'esc' };

/** `cmd+shift+t` → `⌘⇧T` (the sheet's notation: no `+`). */
export function sheetKey(chord) {
  return chord
    .split('+')
    .map((p) => GLYPH[p] ?? p.toUpperCase())
    .join('');
}

const chordOf = (command) => Object.entries(CHORDS).find(([, c]) => c === command)?.[0] ?? null;
const leaderOf = (command) => Object.entries(LEADER).find(([, c]) => c === command)?.[0] ?? null;

/** The key of a bound command, or null when nothing binds it. */
function keyOf(command) {
  const chord = chordOf(command);
  if (chord) return sheetKey(chord);
  const k = leaderOf(command);
  return k ? `${LEADER_KEY} ${k}` : null;
}

/**
 * A row bound to one or more commands (`a / b` rows share one leader key
 * list); dropped when one of them has no binding.
 * @returns {{ keys: string, label: string, commands: string[] } | null}
 */
function bound(label, ...commands) {
  if (commands.length === 1) {
    const keys = keyOf(commands[0]);
    return keys ? { keys, label, commands } : null;
  }
  const leader = commands.map(leaderOf);
  if (leader.some((k) => !k)) return null;
  return { keys: `${LEADER_KEY} ${leader.join(' ')}`, label, commands };
}

/** ⌘1…⌘9 as one row, when all nine are bound. */
function tabNumbers() {
  const commands = [1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => `pane.focus.${n}`);
  if (!commands.every(chordOf)) return null;
  return { keys: '⌘1–9', label: 'Tab 1–9 of the context', commands };
}

/** Pane-local keys: `[keys, label]` pairs as audit §7 lists them. */
const rows = (pairs) => pairs.map(([keys, label]) => ({ keys, label, commands: [] }));

/** @typedef {{ id: string, title: string, rows: { keys: string, label: string, commands: string[] }[] }} Group */

/** @type {Group[]} */
export const GROUPS = [
  {
    id: 'general',
    title: 'General',
    rows: [
      bound('Jump or run a command', 'palette'),
      bound('Jump', 'jump'),
      bound('Settings', 'settings'),
      bound('Keyboard shortcuts', 'shortcuts'),
      bound('Sidebar', 'sidebar'),
      bound('Inspector', 'inspector'),
      bound('Decisions', 'decisions'),
      bound('Ask the Overwatch', 'overwatch.ask'),
    ],
  },
  {
    id: 'workspace',
    title: 'Workspace',
    rows: [
      bound('New session', 'session.new'),
      bound('Add project', 'project.add'),
      bound('Close tab', 'pane.close'),
      bound('Reopen closed tab', 'pane.reopen'),
      tabNumbers(),
      bound('Mission control', 'mission'),
      bound('Week planner', 'planner'),
      bound('Editor (nvim)', 'editor.open'),
      bound('Next waiting agent', 'agent.next-waiting'),
      bound('Pause / unpause agent', 'agent.pause'),
    ],
  },
  {
    id: 'dock',
    title: 'Dock',
    rows: [
      bound('Show / hide', 'dock.toggle'),
      bound('New shell', 'shell.new'),
      bound('Next / previous tab', 'dock.next', 'dock.prev'),
      bound('Into / out of the dock', 'focus.down', 'focus.up'),
    ],
  },
  {
    id: 'agent',
    title: 'Agent pane',
    rows: rows([
      ['⏎', 'Resume (on the strip)'],
      ['⏎', 'Watch: take over'],
      ['esc', 'Watch: back to Mission control'],
    ]),
  },
  {
    id: 'decisions',
    title: 'Decisions & chat',
    rows: rows([
      ['/', 'Search'],
      ['j k', 'Move'],
      ['⏎', 'Open'],
      ['a', 'Approve'],
      ['r', 'Reject'],
      ['e', 'Edit'],
      ['x', 'Dismiss'],
      ['u', 'Undo'],
      ['o', 'Open link'],
      ['t', 'Today'],
      ['[ ]', 'Previous / next day'],
      ['f', 'Filter'],
      ['c', 'Clear digest'],
      ['esc', 'Leave'],
    ]),
  },
  {
    id: 'mission',
    title: 'Mission control',
    rows: rows([
      ['j k', 'Rows'],
      ['h l', 'Chips'],
      ['⏎', 'Open / show card'],
      ['space', 'Details'],
      ['d', 'Decision'],
      ['o', 'Open / PR'],
      ['n', 'Run now'],
      ['⌫', 'Cancel queued'],
      ['i', 'Interrupt'],
      ['p', 'Pause'],
      ['s', 'Suspend'],
      ['r', 'Resume'],
      ['⇧K', 'Kill'],
    ]),
  },
  {
    id: 'feature',
    title: 'Feature',
    rows: rows([
      ['e', 'Open doc in nvim'],
      ['b', 'Tasks'],
      ['w', 'Show source in Work'],
      ['c', 'Canvas'],
      ['p', 'Pause / resume team'],
      ['m', 'Message'],
      ['d', 'Design'],
      ['N', 'Publish to Notion'],
    ]),
  },
  {
    id: 'tasks',
    title: 'Feature › Tasks (focused)',
    rows: rows([
      ['j k h l', 'Move'],
      ['⏎', 'Task menu'],
      ['a', 'Assign'],
      ['m', 'Move to'],
      ['1–5', 'Column'],
      ['e', 'Edit'],
      ['n', 'New task'],
      ['o', 'Open upstream'],
      ['⇧H ⇧L', 'Move card'],
      ['Esc', 'Back to the pane'],
    ]),
  },
  {
    id: 'project',
    title: 'Project',
    rows: rows([
      ['f', 'Fetch'],
      ['p', 'Load PRs'],
      ['t', 'Trust'],
      ['x', 'Remove project'],
    ]),
  },
  {
    id: 'diff',
    title: 'Diff',
    rows: rows([
      ['j k', 'Files'],
      ['r', 'Refresh'],
      ['c', 'Create PR'],
      ['e', 'Open in editor'],
    ]),
  },
  {
    id: 'report',
    title: 'Weekly report',
    rows: rows([
      ['1–4', 'Breakdown'],
      ['R', 'Past reports'],
      ['[ ]', 'Previous / next'],
      ['e', 'Copy as markdown'],
      ['⏎', 'Plan next week'],
      ['esc', 'Week planner'],
    ]),
  },
  {
    id: 'canvas',
    title: 'Team canvas',
    rows: rows([
      ['n', 'Add role'],
      ['c', 'Connect'],
      ['⏎', 'Edit'],
      ['=', 'Fit'],
      ['x', 'Delete'],
      ['u', 'Undo'],
      ['⇧D', 'Duplicate'],
      ['⌃R', 'Redo'],
      ['⇧R', 'Library'],
      ['⌘⏎', 'Launch on feature'],
    ]),
  },
  {
    id: 'design',
    title: 'Design preview',
    rows: rows([
      ['h j k l', 'Move'],
      ['⏎', 'Open'],
      ['space', 'Preview'],
      ['x', 'Ask to export'],
      ['← →', 'Step'],
      ['0 1', 'Fit / 100 %'],
      ['esc', 'Close'],
    ]),
  },
  {
    id: 'planner',
    title: 'Week planner',
    rows: rows([
      ['S', 'Sprint'],
      ['P', 'Pause all'],
      ['R', 'Weekly report'],
      ['e', 'Edit plan'],
      ['O', 'Off-hours'],
      ['a', 'Apply'],
      ['r', 'Keep running'],
      ['u', 'Undo'],
      ['j k', 'Log'],
    ]),
  },
  {
    id: 'settings',
    title: 'Settings lists',
    rows: rows([
      ['j k', 'Rows'],
      ['⏎', 'Edit / connect'],
      ['t', 'Test'],
      ['x', 'Remove'],
      ['n', 'New server'],
      ['a', 'Add'],
      ['⌫', 'Remove'],
      ['⌘O', 'Browse'],
      ['r', 'Retry / sync'],
      ['⌘⏎', 'Save'],
      ['esc', 'Cancel / back to sections'],
    ]),
  },
].map((g) => ({ ...g, rows: g.rows.filter(Boolean) }));

/** Words a search may use for a key glyph ("cmd" finds ⌘). */
const WORDS = [
  ['cmd', '⌘'],
  ['command', '⌘'],
  ['shift', '⇧'],
  ['alt', '⌥'],
  ['option', '⌥'],
  ['opt', '⌥'],
  ['ctrl', '⌃'],
  ['control', '⌃'],
  ['enter', '⏎'],
  ['return', '⏎'],
  ['escape', 'esc'],
];

/** The query with key words turned into their glyphs, lowercased. */
function glyphQuery(q) {
  let s = q.toLowerCase();
  for (const [w, g] of WORDS) s = s.replaceAll(w, g.toLowerCase());
  return s;
}

/**
 * The groups whose rows match `query` by label or key (case-insensitive;
 * "cmd" matches ⌘); a group with no row left is dropped.
 * @param {Group[]} groups
 * @param {string} query
 */
export function filterGroups(groups, query) {
  const q = query.trim().toLowerCase();
  if (!q) return groups;
  // Keys compare without spaces or `+` ("cmd+/" and "cmd /" both find ⌘/).
  const g = glyphQuery(q).replace(/[\s+]+/g, '');
  const hit = (r) => r.label.toLowerCase().includes(q) || r.keys.toLowerCase().replace(/\s+/g, '').includes(g);
  return groups.map((x) => ({ ...x, rows: x.rows.filter(hit) })).filter((x) => x.rows.length);
}
