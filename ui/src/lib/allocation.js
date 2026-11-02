// Project settings › Allocation: the pure logic (share, the table with the
// typed weight, dirty, form → payload, validation, cap bar). The section is
// AllocationSettings.svelte; the commands are in api-contract.md.

export const TIERS = ['critical', 'high', 'normal', 'low'];

/** A weight or cap field's text as a number; null when empty or not a number. */
export function num(text) {
  if (text === '' || text == null) return null;
  const n = Number(text);
  return Number.isFinite(n) ? n : null;
}

/** weight / sum of all weights, in whole percent (rounded per row, never forced to 100). */
export function share(weight, total) {
  const w = Number(weight);
  return total > 0 && Number.isFinite(w) && w > 0 ? Math.round((w / total) * 100) : 0;
}

/** The weight Add pre-fills (the core's proposal; 1 when missing). */
export function proposedDefault(view) {
  const w = Number(view?.proposed_weight);
  return Number.isFinite(w) && w >= 1 ? w : 1;
}

/** The editable form (text fields) for an allocation. */
export function draftOf(a) {
  return {
    weight: String(a.weight),
    tier: a.tier ?? 'normal',
    cap: a.hard_cap_pct == null ? '' : String(a.hard_cap_pct),
  };
}

/** Add: weight = proposal, tier normal, no cap. */
export function addDraft(view) {
  return { weight: String(proposedDefault(view)), tier: 'normal', cap: '' };
}

/** Inline errors by field; empty when valid. */
export function validate(draft) {
  const errors = {};
  const w = num(draft.weight);
  if (w == null || w < 0) errors.weight = 'Weight must be 0 or more';
  const c = num(draft.cap);
  if (draft.cap !== '' && (c == null || c < 0 || c > 100)) errors.cap = 'Hard cap must be between 0 and 100';
  return errors;
}

/** Weight 0 typed = pending removal (state 7). */
export function pendingRemoval(draft) {
  return !!draft && num(draft.weight) === 0;
}

/** What `project_allocation_set` takes: the form, or null to remove. */
export function toForm(draft) {
  if (!draft || pendingRemoval(draft)) return null;
  const cap = num(draft.cap);
  return { weight: num(draft.weight), tier: draft.tier, hard_cap_pct: draft.cap === '' ? null : cap };
}

/** The form the saved allocation stands for (null when not in the plan). */
export function savedForm(view) {
  const a = view?.allocation;
  return a ? { weight: a.weight, tier: a.tier, hard_cap_pct: a.hard_cap_pct ?? null } : null;
}

/** Edited since the load: the payload differs from what is saved. */
export function isDirty(view, draft) {
  return JSON.stringify(toForm(draft)) !== JSON.stringify(savedForm(view));
}

/** Which fields differ from the saved allocation, for the "• changed" marker. */
export function changedFields(view, draft) {
  const a = view?.allocation;
  if (!draft) return {};
  if (!a) return { weight: true, tier: true, cap: true };
  const s = draftOf(a);
  return { weight: draft.weight !== s.weight, tier: draft.tier !== s.tier, cap: draft.cap !== s.cap };
}

/**
 * The "Rest of the plan" table: the plan's allocations in plan order with this
 * project's row in place (own_index) following the typed weight; "other
 * sessions" last. A pending removal or a project not in the plan has no own
 * row; "other sessions" then carries the "+ name" hint.
 */
export function tableRows(view, draft, name) {
  const others = (view?.others ?? []).map((a) => ({ ...a, own: false, label: a.target || a.id, tag: '' }));
  const a = view?.allocation;
  const confirmed = !!view?.plan_confirmed;
  const pending = confirmed && pendingRemoval(draft);
  const rows = [...others];
  if (a && !pending) {
    const live =
      confirmed && draft
        ? { weight: Math.max(0, num(draft.weight) ?? 0), tier: draft.tier }
        : { weight: a.weight, tier: a.tier };
    const own = { ...a, ...live, own: true, label: name, tag: 'this project' };
    const at = Number.isInteger(view.own_index) ? Math.min(view.own_index, rows.length) : Math.max(0, rows.length - 1);
    rows.splice(at, 0, own);
  } else if (confirmed && (!a || pending)) {
    const def = rows.find((r) => r.id === 'default');
    if (def) def.tag = `+ ${name}`;
  }
  const total = rows.reduce((s, r) => s + (Number(r.weight) || 0), 0);
  return rows.map((r) => ({
    ...r,
    share: share(r.weight, total),
    highlight: r.own || (!!r.tag && r.id === 'default'),
  }));
}

/** The own share as typed (live), for "{share}% of the plan"; null when invalid or removed. */
export function ownShare(view, draft, name) {
  if (!draft || pendingRemoval(draft) || validate(draft).weight) return null;
  const rows = view?.allocation ? tableRows(view, draft, name) : null;
  if (rows) return rows.find((r) => r.own)?.share ?? null;
  // Added, not saved yet: the plan's weights plus the typed one.
  const total = (view?.others ?? []).reduce((s, r) => s + (Number(r.weight) || 0), 0) + num(draft.weight);
  return share(num(draft.weight), total);
}

/** Used pp, one decimal. */
export const pp = (v) => (v == null ? '—' : `${Number(v).toFixed(1)} pp`);

/** The cap bar: the track spans 0 → cap/0.8 so the tick sits at 80%. */
export function capBar(used, cap, name) {
  if (cap == null) return null;
  const u = Number(used) || 0;
  const span = cap > 0 ? cap / 0.8 : 1;
  const fill = Math.min(100, Math.max(0, (u / span) * 100));
  const reached = cap === 0 ? u > 0 : u >= cap;
  const near = !reached && cap > 0 && u >= 0.8 * cap;
  const state = reached ? 'red' : near ? 'amber' : 'green';
  const words = reached
    ? `■ cap reached · ${name}'s agents are stopped until the week resets`
    : near
      ? `▲ close to the cap · ${u.toFixed(1)} of ${cap}%`
      : `${cap > 0 ? Math.round((u / cap) * 100) : 0}% of the cap`;
  return { fill, state, words };
}

/** The toast after a save. */
export function savedToast(name, form) {
  return form
    ? `Saved ${name}'s allocation · this week's usage kept`
    : `Removed ${name} from the plan · this week's usage kept`;
}

/** The keys valid right now, for the keys line. */
export function keysFor(view, draft, dirty, loadError) {
  if (loadError) return [['r', 'retry']];
  if (!view) return [];
  if (!view.plan_confirmed) return [['g', 'week planner']];
  const keys = [];
  if (dirty) keys.push(['⌘+⏎', 'save'], ['esc', pendingRemoval(draft) ? 'keep in the plan' : 'cancel']);
  if (!view.allocation && !draft) keys.push(['a', 'add']);
  if (draft && !pendingRemoval(draft)) keys.push(['x', 'remove']);
  if (!dirty) keys.push(['g', 'week planner']);
  return keys;
}
