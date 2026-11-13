// Intake actions (P4.5, spec §3.3, §3.5; notes p4-5 §2.2, §5, §8): what a user press does.
// Every start goes through `intake_prefill` and the existing launch / new-session dialogs;
// nothing here starts work. Mission control stays a pure render of `now`: these run on a key
// or a click only.

import { app, askOverwatch, openProjectSettings, openSettings, openUrl, projects, toast } from './app.svelte.js';
import { UNDO_MS } from './inbox.js';
import { GH_SCOPE_COMMAND, integrationsRow, staleOf, staleToast } from './intake.js';
import { errText, invoke } from './ipc.js';

/** Session-only state of the lane and of planning (notes §1.1, §8.1). */
export const intakeUi = $state({
  showHidden: false, // the header's "n hidden · Show" (lane-wide, per session)
  starting: null, // the item id a dialog is open for ("Starting…")
  lastHide: null, // { item, ref, until } while the Hide toast's Undo shows (5 s)
  plan: null, // the project id the Overwatch input is planning for ("Planning · shop-api")
});

// ---------------------------------------------------------------- dialogs

/**
 * Opens the launch or new-session overlay with a prefill from `intake_prefill`
 * ({launch} or {session}); `intake` is what the submit carries ({item} or {brief_decision});
 * `origin` feeds the "From …" chip, `over` the estimate line, `focus` ('brief') the field.
 */
export function openPrefilled(
  prefill,
  /** @type {{ intake?: any, origin?: any, item?: any, focus?: any }} */ {
    intake,
    origin = null,
    item = null,
    focus = null,
  } = {},
) {
  if (prefill?.launch) app.overlay = { kind: 'launch', prefill: prefill.launch, intake, origin, item, focus };
  else if (prefill?.session)
    app.overlay = { kind: 'new-session', prefill: prefill.session, intake, origin, item, focus };
}

/** Start team run / Start solo from a Ready row (notes §2.2): the dialog, never a start. */
export async function startItem(item, mode) {
  if (intakeUi.starting) return;
  intakeUi.starting = item.id;
  try {
    const r = await invoke('intake_prefill', { item: item.id, mode });
    openPrefilled(r, {
      intake: { item: item.id },
      origin: { kind: 'intake', item: item.id, source: item.source, ref: item.ref, url: item.url ?? null },
      item,
    });
    if (!app.overlay || app.overlay.item?.id !== item.id) intakeUi.starting = null;
  } catch (e) {
    intakeUi.starting = null;
    const stale = staleOf(e);
    toast(stale ? staleToast(stale, item.ref) : errText(e), { kind: 'error' });
  }
}

/** A brief card's Launch / Edit (notes §8.3): the dialog with the proposed mode. */
export async function startBrief(decision, mode, { edit = false } = {}) {
  try {
    const r = await invoke('intake_prefill', { brief_decision: decision.id, mode });
    openPrefilled(r, {
      intake: { brief_decision: decision.id },
      origin: { kind: 'brief', decision: decision.id, title: decision.title ?? decision.payload?.title ?? null },
      focus: edit ? 'brief' : null,
    });
    return true;
  } catch (e) {
    const stale = staleOf(e);
    toast(stale ? staleToast(stale) : `Could not open the dialog: ${errText(e)}`, { kind: 'error' });
    return false;
  }
}

/** The dialog closed without a start: the row's "Starting…" tag clears. */
export const endStarting = () => {
  intakeUi.starting = null;
};

// ---------------------------------------------------------------- the lane

/** Refresh for the shown projects (one call per project, so a filter is respected). */
export async function refreshIntake(projectIds) {
  // `undefined` is every project; an empty list (a filter with no source) is none.
  if (projectIds && !projectIds.length) return;
  try {
    await Promise.all((projectIds ?? [null]).map((project) => invoke('intake_refresh', project ? { project } : {})));
  } catch (e) {
    toast(`Could not refresh: ${errText(e)}`, { kind: 'error' });
  }
}

/** Hide ("Not now", x): the toast follows the core's answer, with Undo for 5 s (notes §2.2). */
export async function hideItem(item) {
  try {
    await invoke('intake_hide', { item: item.id });
  } catch (e) {
    return toast(`Could not hide ${item.ref}: ${errText(e)}`, { kind: 'error' });
  }
  const hidden = { item, until: Date.now() + UNDO_MS };
  intakeUi.lastHide = hidden;
  toast(`Hidden ${item.ref}. It comes back if it changes at the source.`, {
    ms: UNDO_MS,
    action: { label: 'Undo u', run: () => undoHide(hidden) },
  });
}

export async function unhideItem(item) {
  try {
    await invoke('intake_unhide', { item: item.id });
    toast(`${item.ref} is back in Ready to start.`);
  } catch (e) {
    toast(`Could not unhide ${item.ref}: ${errText(e)}`, { kind: 'error' });
  }
}

/** `u` in the pane, or the toast's button, within the 5 s. */
export async function undoHide(h = intakeUi.lastHide) {
  // `lastHide` is a state proxy: compare by value, not identity.
  const current = !!h && intakeUi.lastHide?.until === h.until && intakeUi.lastHide?.item?.id === h.item?.id;
  if (!h || h.until < Date.now()) {
    // Expired: `u` goes back to the pane's other keys.
    if (!h || current) intakeUi.lastHide = null;
    return false;
  }
  if (current) intakeUi.lastHide = null;
  await unhideItem(h.item);
  return true;
}

/** Open source (o): GitHub / Linear in the browser, a repo doc in the editor (notes §3). */
export async function openSource(item, openEditor) {
  if (item.url) return openUrl(item.url);
  const p = projects().find((x) => x.id === item.project);
  if (p) openEditor?.(item.ref, null, { projectId: p.id });
}

// ---------------------------------------------------------------- errors, set up

/** The error line's Fix (notes §4.2): where `fix` sends the user; Copy command for `gh_scope`. */
export async function fixSource(src, refresh = () => refreshIntake([src.project])) {
  const code = src.error?.code;
  const fix = src.error?.fix;
  if (fix === 'gh_scope' || code === 'gh_scope') {
    try {
      await navigator.clipboard.writeText(GH_SCOPE_COMMAND);
      toast('Copied. Run it in a terminal, then Refresh.');
    } catch {
      toast(`Run it in a terminal, then Refresh: ${GH_SCOPE_COMMAND}`);
    }
  } else if (fix === 'integrations') openSettings('integrations', integrationsRow(code));
  else if (fix === 'settings') openProjectSettings(src.project, 'tracker');
  else return refresh();
}

/** Set up intake (e): Project settings › Intake. */
export const setUpIntake = (project) => openProjectSettings(project, 'tracker');

// ---------------------------------------------------------------- Plan with the Overwatch (notes §8.1)

const draftFor = (name) => `Plan a feature for ${name}: `;
const isPlanDraft = (t) => /^Plan a feature for .+: $/.test(t);

/**
 * Focuses the Overwatch input with the draft "Plan a feature for shop-api: " and tags the
 * conversation as planning (`chat_send` then carries `context: {plan}`). The draft is replaced
 * only when empty or a previous planning draft; otherwise the line is appended.
 */
export function planWithOverwatch(projectId) {
  const p = projects().find((x) => x.id === projectId);
  if (!p) return;
  app.overlay = null;
  const cur = app.chatDraft ?? '';
  const line = draftFor(p.name);
  app.chatDraft = !cur.trim() || isPlanDraft(cur) ? line : `${cur.replace(/\s+$/, '')}\n${line}`;
  intakeUi.plan = p.id;
  askOverwatch();
}

/** The planning context a send carries, or undefined (the tag's ✕ clears it). */
export const planContext = () => (intakeUi.plan ? { plan: intakeUi.plan } : undefined);
export const clearPlan = () => {
  intakeUi.plan = null;
};

/** Planning messages by entry id → project id: the user's entry shows "planning · shop-api" (notes §8.2). */
export const plannedEntries = $state({});

/**
 * Plan with the Overwatch for one of `projectIds` (notes §8.1): one id goes straight to the draft,
 * several open the chooser listing exactly those, in that order (the filter's first).
 */
export function planWithOverwatchIn(projectIds) {
  const ids = [...new Set(projectIds ?? [])].filter((id) => projects().some((p) => p.id === id));
  if (ids.length === 1) return planWithOverwatch(ids[0]);
  if (ids.length) app.overlay = { kind: 'plan-project', ids };
}
