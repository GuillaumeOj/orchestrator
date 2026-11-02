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

/** An empty cap field (a number input gives null once cleared). */
const noCap = (c) => c === '' || c == null;

/** Inline errors by field; empty when valid. */
export function validate(draft) {
  const errors = {};
  const w = num(draft.weight);
  if (w == null || w < 0) errors.weight = 'Weight must be 0 or more';
  // The cap is ignored when the weight is 0 (removal).
  const c = num(draft.cap);
  if (w !== 0 && !noCap(draft.cap) && (c == null || c < 0 || c > 100))
    errors.cap = 'Hard cap must be between 0 and 100';
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
  return { weight: num(draft.weight), tier: draft.tier, hard_cap_pct: noCap(draft.cap) ? null : cap };
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
  return { weight: draft.weight !== s.weight, tier: draft.tier !== s.tier, cap: String(draft.cap ?? '') !== s.cap };
}

/**
 * The "Rest of the plan" table: the plan's allocations in plan order with this
 * project's row in place (own_index, else before "other sessions") following
 * the typed weight; "other sessions" last. A project without an own row (not in
 * the plan, or pending removal) puts "+ name" on "other sessions"; a pending
 * removal also moves the project's usage there.
 */
export function tableRows(view, draft, name) {
  const confirmed = !!view?.plan_confirmed;
  const a = view?.allocation;
  const pending = confirmed && !!a && pendingRemoval(draft);
  const rows = (view?.others ?? []).map((o) => ({ ...o, own: false, label: o.target || o.id, tag: '' }));
  const editing = confirmed && !!draft;
  if (!confirmed || (a && !pending) || (!a && editing)) {
    const base = a ?? { id: view.project, tier: 'normal', weight: 0, used_pct: 0 };
    const live = editing ? { weight: Math.max(0, num(draft.weight) ?? 0), tier: draft.tier } : {};
    const own = { ...base, ...live, own: true, label: name, tag: 'this project' };
    const def = rows.findIndex((r) => r.id === 'default');
    const at = Number.isInteger(view.own_index) ? Math.min(view.own_index, rows.length) : def >= 0 ? def : rows.length;
    rows.splice(at, 0, own);
  } else if (confirmed) {
    const def = rows.find((r) => r.id === 'default');
    if (def) {
      def.tag = `+ ${name}`;
      if (pending) def.used_pct = (Number(def.used_pct) || 0) + (Number(a.used_pct) || 0);
    }
  }
  const total = rows.reduce((t, r) => t + (Number(r.weight) || 0), 0);
  return rows.map((r) => ({ ...r, share: share(r.weight, total), highlight: r.own || !!r.tag }));
}

/** The cap bar: the track spans 0 → cap/0.8 so the tick sits at 80%. */
export function capBar(used, cap, name) {
  if (cap == null) return null;
  const u = Number(used) || 0;
  const span = cap > 0 ? cap / 0.8 : 1;
  const reached = u >= cap;
  const fill = reached ? 100 : Math.min(100, Math.max(0, (u / span) * 100));
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
export function keysFor(view, draft, dirty, invalid, loadError) {
  if (loadError) return [['r', 'retry']];
  if (!view) return [];
  if (!view.plan_confirmed) return [['g', 'week planner']];
  const pending = !!view.allocation && pendingRemoval(draft);
  const keys = [];
  if (dirty) {
    if (!invalid) keys.push(['⌘+⏎', 'save']);
    keys.push(['esc', pending ? 'keep in the plan' : 'cancel']);
  }
  if (!view.allocation && !draft) keys.push(['a', 'add']);
  if (draft && !pending) keys.push(['x', 'remove']);
  if (!dirty) keys.push(['g', 'week planner']);
  return keys;
}
