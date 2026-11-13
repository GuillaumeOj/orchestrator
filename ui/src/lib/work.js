// Entry to the Work browser (improve-intake notes §1.2): every caller (sidebar, Project pane, palette,
// leader ⌘; b, Feature "Show in Work", run inspector) goes through `openWork`. One tab per scope:
// the same scope focuses its tab (and lands the reveal / search there), another opens a new tab.
import {
  addPane,
  app,
  confirm,
  featureBySlug,
  findPane,
  openFeature,
  openSession,
  openUrl,
  showPane,
  toast,
} from './app.svelte.js';
import { errText, invoke } from './ipc.js';
import { scopeKey } from './workmodel.js';

/** The pane spec's scope: sorted, unique ids, or null for every project. */
export const scopeOf = (ids) => (ids?.length ? [...new Set(ids.map(String))].sort() : null);

const sameScope = (s, list) =>
  s.type === 'work' && (s.projects == null) === (list == null) && scopeKey(s.projects) === scopeKey(list);

/**
 * Opens Work on `projects` (ids; null = all), revealing `reveal` (a WorkNode id) when given, or with
 * the search focused (`find`, the palette's "Find work item…").
 * @param {{ projects?: string[] | null, reveal?: string | null, find?: boolean }} [scope]
 */
export function openWork({ projects = null, reveal = null, find = false } = {}) {
  const list = scopeOf(projects);
  // A one-shot request the pane clears; `land` is never saved with the layout.
  const land = reveal || find ? { reveal, find, seq: Date.now() } : null;
  const existing = findPane((s) => sameScope(s, list));
  if (existing) {
    if (land) app.panes[existing] = { ...app.panes[existing], land };
    return showPane(existing);
  }
  addPane({ type: 'work', projects: list, ...(land ? { land } : {}) });
}

/**
 * Run as program (`p`): the plan dialog of `roots` (one node, or ≥ 2 siblings), improve-intake
 * notes §6. `title`: what the dialog names before the plan arrives ("#12 Checkout v2").
 * @param {string[]} roots
 * @param {{ title?: string }} [opts]
 */
export function openProgramPlan(roots, { title = '' } = {}) {
  if (!roots?.length) return;
  app.overlay = { kind: 'program-plan', roots: [...roots], title };
}

/** Open program (`p` on a program root, the chip, the palette, a card): the Program view, one tab per program. */
export function openProgram(id, title = '', { add = null } = {}) {
  if (!id) return;
  const row = app.now.programs?.[id];
  const name = title || row?.title || '';
  // A one-shot request the pane clears: the row editor of a new upstream item (a card's Add to program).
  const land = add ? { add, seq: Date.now() } : null;
  const existing = findPane((s) => s.type === 'program' && s.programId === id);
  if (existing) {
    if (land) app.panes[existing] = { ...app.panes[existing], land };
    return showPane(existing);
  }
  addPane({
    type: 'program',
    programId: id,
    programTitle: name,
    projectId: row?.project ?? null,
    ...(land ? { land } : {}),
  });
}

/** The teams a program feature can take in `project` (the ones without issues), as `{id, name}`; [] on an error. */
export function programTeams(project) {
  return invoke('teams_view', { project })
    .then((v) => (v?.teams ?? []).filter((t) => !t.issues?.length).map((t) => ({ id: t.team.id, name: t.team.name })))
    .catch(() => []);
}

/** `program_update(id, patch)`: the Program as the core stored it; a refusal toasts and gives null. */
export async function updateProgram(id, patch) {
  try {
    return await invoke('program_update', { id, patch });
  } catch (e) {
    toast(`Could not update the program: ${errText(e)}`, { kind: 'error' });
    return null;
  }
}

/** Pause / resume (`p`): confirm-less, with the §7.2 toasts. */
export async function togglePause(p) {
  const pause = p.status !== 'paused';
  const r = await updateProgram(p.id, pause ? { pause: true } : { resume: true });
  if (r) toast(pause ? `${p.title} paused. Running features continue.` : `${p.title} resumed.`);
  return r;
}

/** Cancel program (`⇧K`), after the §7.2 confirm. */
/** @param {any} p @param {(r: any) => void} [then] */
export function cancelProgram(p, then = (_r) => {}) {
  confirm(
    `Cancel ${p.title}?`,
    'Running features keep running as normal features; waiting ones are dropped.',
    async () => then(await updateProgram(p.id, { cancel: true })),
    'Cancel program',
    'Keep',
  );
}

/** Skip a feature (`x`), after the §7.2 confirm. */
/** @param {any} p @param {any} f @param {(r: any) => void} [then] */
export function skipFeature(p, f, then = (_r) => {}) {
  confirm(
    `Skip ${f.slug}?`,
    'Features after it no longer wait for it.',
    async () => then(await updateProgram(p.id, { skip: f.key })),
    'Skip',
    'Keep',
  );
}

/** `o` on a program row: its source item upstream (the first root), in the browser. */
export async function openProgramSource(id) {
  try {
    const p = await invoke('program', { id });
    await openNodeLink(p?.root?.[0], p?.root_ref ?? 'The source item');
  } catch (e) {
    toast(`Could not open the link: ${errText(e)}`, { kind: 'error' });
  }
}

/** A node's link upstream, in the browser (`o` on a program's rows); `label` names it when it has none. */
export async function openNodeLink(id, label) {
  try {
    const node = id ? await invoke('work_node', { id, opts: { children: false } }) : null;
    if (node?.url) return openUrl(node.url);
    toast(`${label} has no link to open.`);
  } catch (e) {
    toast(`Could not open the link: ${errText(e)}`, { kind: 'error' });
  }
}

/** A program feature's Feature pane (by slug in the program's project), else its solo session, else `fallback`. */
export function openProgramFeature(p, f, fallback = () => openProgram(p.id, p.title)) {
  const feat = featureBySlug(p.project, f.slug);
  if (feat) return openFeature(feat.id);
  if (f.agent) return openSession(f.agent);
  fallback();
}

/** The Program view's tab title. */
export const programTabTitle = (title) => `${title || 'Program'} · program`;
