// Project settings › Tracker (improve-intake design notes §10, api-contract §1). Pure: tested in
// test/tracker-settings.test.js. The form is a TrackerSettings (contract §1) edited in place.

import { errorInfo } from './intake.js';

export const KINDS = [
  { id: 'local', label: 'Local' },
  { id: 'github', label: 'GitHub' },
  { id: 'linear', label: 'Linear' },
];

export const DEFAULT_GLOB = '.orchestrator/backlog/*.md';
export const REFRESH = [
  { value: 300, label: 'Every 5 min' },
  { value: 600, label: 'Every 10 min' },
  { value: 1800, label: 'Every 30 min' },
  { value: 3600, label: 'Every 1 h' },
];
export const STATE_TYPES = [
  { id: 'backlog', label: 'Backlog' },
  { id: 'unstarted', label: 'Unstarted' },
];
/** `types` with `id` ticked or not: the known types in their order, types the form does not list (a saved `started`) kept. */
export function toggleType(types, id, on) {
  const next = on ? [...new Set([...types, id])] : types.filter((t) => t !== id);
  const order = STATE_TYPES.map((x) => x.id);
  return [...order.filter((t) => next.includes(t)), ...next.filter((t) => !order.includes(t))];
}

export const CYCLES = [
  { value: 'current', label: 'Current' },
  { value: 'next', label: 'Next' },
  { value: null, label: 'Any' },
];

/** The Tracker of a kind, with its defaults (a switch of kind starts from these). */
export const trackerDefaults = (kind) =>
  kind === 'github'
    ? { kind, repo: null, project_board: null }
    : kind === 'linear'
      ? { kind, teams: [], projects: null }
      : { kind: 'local', backlog_glob: DEFAULT_GLOB };

/** The Ready filter of a kind, with its defaults (notes §10.3). GitHub's `ready` label is added once the repo's labels are known. */
function readyDefaults(kind) {
  if (kind === 'github')
    return {
      kind,
      labels: [],
      milestone: null,
      board_status: null,
      issue_type: null,
    };
  if (kind === 'linear') return { kind, state_types: ['unstarted'], label: null, project: null, cycle: 'current' };
  return { kind: 'local' };
}

/** A fresh form for `kind`, from the form it replaces: only `write_back` / `refresh_secs` carry over. */
export function switchKind(form, kind) {
  return {
    ...form,
    tracker: trackerDefaults(kind),
    ready: readyDefaults(kind),
    // The board status / Linear state belong to a kind: they do not carry over.
    write_back: { ...form.write_back, board_status: null, linear_state: null },
    task_states: null,
    group_by_milestone: kind === 'github' ? form.group_by_milestone : true,
  };
}

/** "acme/shop" from a project's remote url (github.com only), else null. */
export function repoOfRemote(url) {
  const m = /github\.com[:/]([^/\s]+\/[^/\s]+?)(?:\.git)?$/.exec(url ?? '');
  return m ? m[1] : null;
}

const REPO = /^[^/\s]+\/[^/\s]+$/;

/** A glob the core refuses: absolute, or with a `..` path component (`v1..v2` is fine). */
export const hasParentPart = (g) => g.startsWith('/') || g.split('/').includes('..');

/** Client rules (the same fields the core validates), field → message in the notes' words; {} when valid. */
export function problems(form) {
  const p = /** @type {Record<string, string>} */ ({});
  const t = form.tracker;
  if (t.kind === 'github') {
    if (t.repo?.trim() && !REPO.test(t.repo.trim())) p.repo = 'Use owner/name, e.g. acme/shop.';
  } else if (t.kind === 'linear') {
    if (!t.teams.length) p.teams = 'Pick at least one team.';
    if (t.projects && !t.projects.length) p.projects = 'Pick at least one project.';
  } else {
    const g = (t.backlog_glob ?? '').trim();
    if (g && hasParentPart(g)) p.backlog_glob = 'Use a path inside the repo: no leading `/`, no `..`.';
  }
  return p;
}

/** The core's field error, in the notes' words. */
export function fieldCopy(field, message = '') {
  switch (field) {
    case 'repo':
      return 'Use owner/name, e.g. acme/shop.';
    case 'teams':
      return 'Pick at least one team.';
    case 'backlog_glob':
      return /\.\.|leading|\//.test(message) && !/parse/i.test(message)
        ? 'Use a path inside the repo: no leading `/`, no `..`.'
        : "The glob doesn't parse.";
    case 'ready':
      return 'Ready filter does not match the tracker';
    default:
      return message;
  }
}

/** A command error as { code, field, message }: an object, or JSON text. */
export function parseErr(e, text) {
  if (e && typeof e === 'object' && ('code' in e || 'field' in e)) return e;
  try {
    const j = JSON.parse(typeof e === 'string' ? e : (e?.message ?? ''));
    if (j && typeof j === 'object') return j;
  } catch {}
  return { message: text(e) };
}

export const nameOf = (x) =>
  x && typeof x === 'object' ? (x.title ?? x.name ?? x.label ?? String(x.id ?? '')) : String(x);

/** Option lists of `tracker_options` (strings or objects) as Select options. */
export const optionList = (xs, any = null) => [
  ...(any ? [{ value: '', label: any }] : []),
  ...(xs ?? []).map((x) => ({ value: nameOf(x), label: nameOf(x) })),
];

/** What the Ready filter lists, in a line (notes §10.3). `projectName` resolves a Linear project id. */
export function readySummary(ready, projectName = (id) => id) {
  if (ready.kind === 'local') return 'Every backlog file not marked done.';
  if (ready.kind === 'github') {
    const parts = [];
    if (ready.labels.length) parts.push(`labelled ${ready.labels.join(', ')}`);
    if (ready.milestone) parts.push(`in milestone ${ready.milestone}`);
    if (ready.board_status) parts.push(`with board status ${ready.board_status}`);
    if (ready.issue_type) parts.push(`of type ${ready.issue_type}`);
    return parts.length ? `Open issues ${parts.join(', ')}` : 'Nothing: the lane stays empty until you pick something';
  }
  const types = STATE_TYPES.filter((s) => ready.state_types.includes(s.id)).map((s) => s.label);
  const parts = [];
  if (ready.label) parts.push(`labelled ${ready.label}`);
  if (ready.project) parts.push(`in project ${projectName(ready.project)}`);
  if (ready.cycle) parts.push(`in the ${ready.cycle} cycle`);
  // No state type ticked: the core still lists what a label or project selects.
  if (!types.length)
    return parts.length ? `Issues ${parts.join(', ')}` : 'Nothing: the lane stays empty until you pick something';
  return `${types.join(' and ')} issues${parts.length ? ` ${parts.join(', ')}` : ''}`;
}

/** The tracker in a few words ("acme/shop", "Linear ENG", "Local"), for sentences. */
export function trackerName(t, known = null) {
  if (t.kind === 'github') return t.repo || known || "this project's repository";
  if (t.kind === 'linear') return `Linear ${t.teams.join(', ')}`;
  return 'Local';
}

/** The write-back list under the switch (notes §10.4, the owner of the sentences). */
/** `target`: the name of the board Status / Linear state items move to (default "In progress"). */
export function writeBackLines(form, known = null, target = null) {
  const t = form.tracker;
  const wb = form.write_back;
  const who = trackerName(t, known);
  const assign = wb.assign_me ? 'it is assigned to you, ' : '';
  const comment = 'a comment "Started in Orchestrator"';
  const status = target ?? wb.board_status ?? 'In progress';
  const start =
    t.kind === 'github' && !t.project_board
      ? `When you start an item, ${assign}it gets the label ${wb.running_label || 'orch:running'} and ${comment}.`
      : `When you start an item, ${assign}it moves to ${t.kind === 'github' ? `${status} on the board` : status} and gets ${comment}.`;
  return [
    start,
    'Team tasks are created as sub-issues of the item, labelled orch.',
    t.kind === 'github'
      ? 'Pull requests say Closes #13, so merging closes the item.'
      : 'Pull requests link the issue; merging moves it to Done.',
    "A program's parent closes when all its items are done.",
    `Nothing else is changed, and only items of ${t.kind === 'github' ? who : `team${t.teams.length === 1 ? '' : 's'} ${t.teams.join(', ')}`}.`,
  ];
}

/** Default column of a Linear state: what the core sends (`default_column`), else by its type (the mapping moved from Task backend). */
export function stateColumn(s) {
  // The core's own default wins (`default_column`; null = it maps nowhere).
  if ('default_column' in s) return s.default_column ?? null;
  if (s.type === 'started') return /review/i.test(s.name) ? 'review' : 'in_progress';
  return (
    { backlog: 'backlog', triage: 'backlog', unstarted: 'ready', completed: 'done', canceled: 'done' }[s.type] ??
    'backlog'
  );
}

/** The Linear states as mapping rows: the saved column, else the default. */
export function stateRows(states, saved) {
  return (states ?? []).map((s) => {
    const def = stateColumn(s);
    const column = saved?.[s.id] ?? def;
    return { id: s.id, name: s.name, def, column };
  });
}

/** `task_states` to save: only the rows off their default; null when none. */
export function statesToSave(rows) {
  const out = {};
  for (const r of rows) if (r.column && r.column !== r.def) out[r.id] = r.column;
  return Object.keys(out).length ? out : null;
}

/** One preview row's right-hand text: "in #12 Checkout v2" / "in v2 · ready" / "no milestone". */
export function sampleWhere(node, kind) {
  const crumbs = node.breadcrumb ?? [];
  let where = '';
  if (crumbs.length) {
    const last = crumbs.at(-1);
    where =
      kind === 'github' && !node.upstream?.milestone
        ? `in ${last.ref} ${last.title}`
        : `in ${crumbs.map((c) => c.title).join(' › ')}`;
  } else if (node.upstream?.milestone) where = `in ${node.upstream.milestone}`;
  else if (kind === 'github') where = 'no milestone';
  return [where, node.ready ? 'ready' : ''].filter(Boolean).join(' · ');
}

/** A source error in words, with the Tracker fixes of notes §10.7 on top of the shared table. */
export function trackerError(error, kind, name = null) {
  const code = error?.code;
  const info = errorInfo(error, { type: kind });
  switch (code) {
    case 'linear_complexity':
      return {
        ...info,
        text: 'Linear said the request was too large. Work shows what is cached.',
        fix: 'limit_projects',
        fixLabel: 'Limit the tracker to fewer projects',
      };
    case 'gh_sub_issues':
      return {
        ...info,
        text: `GitHub didn't return sub-issues${name ? ` for ${name}` : ''}. Work shows issues without hierarchy.`,
        fix: 'retry',
        fixLabel: 'Retry',
      };
    case 'gh_write':
    case 'linear_write':
      return {
        ...info,
        text:
          code === 'gh_write'
            ? `GitHub refused the change: your login can't write to ${name ?? 'this repository'}.`
            : `Linear refused the change: your key can't write to ${name ?? 'these teams'}.`,
        fix: 'turn_off_write_back',
        fixLabel: 'Turn off write-back',
      };
    default:
      return info;
  }
}

/** Migration notice copy (notes §10.6): { tone, text, actions } for the notice of the contract. */
export function noticeInfo(n, form, { project = 'this project', known = null } = {}) {
  const kept = trackerName(n.kept, known);
  const alt = n.alternative ? trackerName(n.alternative, known) : null;
  const src = n.from?.intake_source ?? {};
  const filter = intakeFilterText(src);
  const readyText = readySummary(form?.ready ?? { kind: 'local' }).replace(/^Open issues /, '');
  switch (n.kind) {
    case 'merged':
      return {
        tone: 'info',
        text: `Task backend and Intake are now one setting: the tracker. ${kept} is used for both, and your Ready filter (${filter || readyText}) is kept.`,
        actions: ['got_it'],
      };
    case 'different_repo':
      return {
        tone: 'info',
        text: `Task backend used ${kept} and Intake used ${alt}. The tracker keeps ${kept}; the Intake ${filter || 'filter'} is now its Ready filter. ${alt} is no longer read.`,
        actions: ['got_it', 'use_alternative'],
        altLabel: `Use ${alt} instead`,
      };
    case 'mismatch': {
      const intakeSide =
        src.type === 'repo_docs'
          ? "Intake's repo docs folder"
          : `Intake read ${src.type === 'linear' ? 'Linear' : 'GitHub'} ${alt}`;
      const tasks = n.kept.kind === 'local' ? 'tasks were local' : `tasks were in ${kept}`;
      return {
        tone: 'warning',
        text: `${intakeSide} but ${tasks}. Pick the tracker for ${project}:`,
        choices: [
          {
            id: 'alternative',
            label:
              n.alternative.kind === 'local'
                ? 'Local backlog'
                : `${n.alternative.kind === 'linear' ? 'Linear' : 'GitHub'} ${alt.replace(/^Linear /, '')}`,
            tracker: n.alternative,
          },
          {
            id: 'kept',
            label:
              n.kept.kind === 'local'
                ? 'Local'
                : `${n.kept.kind === 'linear' ? 'Linear' : 'GitHub'} ${kept.replace(/^Linear /, '')}`,
            tracker: n.kept,
          },
        ],
        until: `Until you pick, Work shows ${kept}.`,
        actions: ['use_this'],
      };
    }
    default:
      return {
        tone: 'info',
        text: "Intake's repo docs folder is now the Local tracker's backlog.",
        actions: ['got_it'],
      };
  }
}

/** The old intake filter in words: "label ready", "milestone v2", "project acme #4". */
function intakeFilterText(src) {
  const f = src.filter;
  if (src.type === 'github' && f) {
    if (f.kind === 'label') return `label ${(f.names ?? []).join(', ')}`;
    if (f.kind === 'milestone') return `milestone ${f.title}`;
    if (f.kind === 'project') return `project ${f.owner} #${f.number}`;
  }
  if (src.type === 'linear' && src.label) return `label ${src.label}`;
  return '';
}
