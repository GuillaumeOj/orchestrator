// Intake (P4.5, FR-1312; api-contract §1–3, §7; notes p4-5 §1–§7): the copy and
// the pure rules of the Ready to start lane, its rows, the source errors and the
// prefilled dialogs. Pure: tested in test/intake.test.js. Nothing here reads the
// app state, the clock or the core; callers pass `now` and the data.

import { clock, plural, startOfDay } from './format.js';

/** Source names in copy (notes: GitHub, Linear, Repo docs). */
export const SOURCE_NAME = { github: 'GitHub', linear: 'Linear', repo_docs: 'Repo docs', none: 'No source' };

/** Start button words (notes §2.2, §2.3) and their keys. */
export const MODE_LABEL = {
  team: { long: 'Start team run', short: 'Team run', key: 't' },
  solo: { long: 'Start solo', short: 'Solo', key: 'a' },
};

// ---------------------------------------------------------------- estimates (notes §2.1)

/** "4–7", "6", "< 1", "< 1–3": a range in pp of the week without decimals; '' without figures. */
export function ppRange(low, high) {
  if (low == null || high == null || Number.isNaN(+low) || Number.isNaN(+high)) return '';
  if (high < 1) return '< 1';
  const [lo, hi] = [Math.round(low), Math.round(high)];
  if (lo === 0) return hi <= 1 ? '1' : `< 1–${hi}`;
  return lo === hi ? `${lo}` : `${lo}–${hi}`;
}

const approx = (r) => (r.startsWith('<') ? r : `≈ ${r}`);

/** The suggested mode's estimate (`team` | `solo`) of an item, from its `mode` (else team when it can). */
export function modeOf(item) {
  // A team run needs a team (F24): without one, solo is the suggestion whatever the front matter asks.
  if (item?.can_team === false) return 'solo';
  return item?.mode === 'solo' || item?.mode === 'team' ? item.mode : item?.can_team ? 'team' : 'solo';
}

/**
 * The row's estimate (§2.1): "≈ 4–7 pp week · team", "rough ≈ 4–8 pp week · team",
 * "no estimate". `compact` (≤ 1000 px, §2.3) drops "week".
 */
export function estimateText(item, mode = modeOf(item), { compact = false } = {}) {
  const e = item?.estimate?.[mode];
  const r = e ? ppRange(e.low_pp, e.high_pp) : '';
  if (!r) return 'no estimate';
  return `${e.basis === 'default' ? 'rough ' : ''}${approx(r)} pp${compact ? '' : ' week'} · ${mode}`;
}

/** The tooltip of the estimate (§2.1). */
export function estimateTip(item, mode = modeOf(item)) {
  const e = item?.estimate?.[mode];
  if (!e) return '';
  const range = ppRange(e.low_pp, e.high_pp);
  if (e.basis === 'default') return `No finished runs yet: using the default (${mode === 'team' ? '6' : '1.5'} pp)`;
  return `${mode === 'team' ? 'Team run' : 'Solo session'}: ${approx(range)} pp of the week. Basis in details.`;
}

/** Whether a mode's high end is over the room (F7: the UI computes the other mode itself). */
export function overRoom(est, mode) {
  const e = est?.[mode];
  const room = est?.room_pp;
  return !!e && room != null && e.high_pp > room;
}

/** Line 2's warning (§2.1, F6): "over this week's room (3 pp left)"; '' when not over. */
export function roomWarning(item, projectName = null) {
  const est = item?.estimate;
  if (!est || !item?.estimate?.over_room || est.room_pp == null) return '';
  const left = est.room_pp < 0.5 ? 'none left' : `${Math.round(est.room_pp)} pp left`;
  const scope =
    est.room_scope === 'allocation' && projectName ? `over ${projectName}'s room this week` : "over this week's room";
  return `${scope} (${left})`;
}

/** One decimal for the details (§3). */
const one = (v) => Number(v).toFixed(1);

/** Details: "from 7 runs in shop-api · size M" and its kin, by basis (§3). */
export function basisText(e, mode, size, projectName = 'this project') {
  if (!e) return '';
  const sz = size ? `size ${size}` : 'size M (not set)';
  const unit = mode === 'team' ? 'run' : 'session';
  if (e.basis === 'default') return `rough: no history yet · ${sz} (default ${mode === 'team' ? '6' : '1.5'} pp)`;
  const n = e.samples ?? 0;
  const where = e.basis === 'project' ? `in ${projectName}` : 'across projects';
  return `from ${plural(n, unit)} ${where}${n === 1 ? ', ± 40 %' : ''} · ${sz}`;
}

/** Details: "≈ 4.1–6.8 pp of the week". */
export const exactRange = (e) => (e ? `≈ ${one(e.low_pp)}–${one(e.high_pp)} pp of the week` : '');

/** Dialog line (§5.2): "≈ 4–7 pp of the week for a team run · over this week's room (3 pp left)". */
export function dialogEstimate(item, mode, projectName = null) {
  const e = item?.estimate?.[mode];
  const r = e ? ppRange(e.low_pp, e.high_pp) : '';
  if (!r) return '';
  const base = `${approx(r)} pp of the week for a ${mode === 'team' ? 'team run' : 'solo session'}`;
  const over = overRoom(item.estimate, mode)
    ? roomWarning({ estimate: { ...item.estimate, over_room: true } }, projectName)
    : '';
  return over ? `${base} · ${over}` : base;
}

// ---------------------------------------------------------------- refs, labels

const stripGlob = (paths) => {
  if (paths.length < 2) return '';
  const parts = paths.map((p) => p.split('/').slice(0, -1));
  const first = parts[0];
  let n = 0;
  while (n < first.length && parts.every((p) => p[n] === first[n])) n++;
  return n ? `${first.slice(0, n).join('/')}/` : '';
};

/**
 * The canonical short ref (§2): `#123` and `ENG-42` as given; a repo doc without the
 * folder all of the project's docs share (`checkout-v2.md`), else its last two segments
 * with a leading `…/`. `siblings`: the project's repo-docs items.
 */
export function shortRef(item, siblings = []) {
  const ref = String(item?.ref ?? '');
  if (item?.source !== 'repo_docs') return ref;
  const all = siblings.filter((s) => s.source === 'repo_docs').map((s) => String(s.ref));
  if (all.length < 2) {
    // One doc: the file name when its folder is the default backlog, else the last two segments.
    const segs = ref.split('/');
    return segs.length <= 2
      ? ref
      : /(^|\/)backlog\/[^/]+$/.test(ref)
        ? segs.slice(-1)[0]
        : `…/${segs.slice(-2).join('/')}`;
  }
  const prefix = stripGlob(all);
  if (prefix && ref.startsWith(prefix)) return ref.slice(prefix.length);
  const segs = ref.split('/');
  return segs.length <= 2 ? ref : `…/${segs.slice(-2).join('/')}`;
}

/**
 * The count in "+ n more": "12", "1990+" when the refresh stopped at the read bound
 * (`partial`: `more` counts only what was read), "" when partial with nothing counted.
 */
export const moreCount = (more, partial) => (partial ? (more > 0 ? `${more}+` : '') : String(more));

/** Up to two labels as chips (+n), without `size:*` / `size/*` and `orch*` (§2). */
export function shownLabels(item) {
  const all = (item?.labels ?? []).filter((l) => !/^size[:/]/i.test(l) && !/^orch(:|$)/i.test(l));
  return { chips: all.slice(0, 2), more: Math.max(0, all.length - 2) };
}

/** The accessible name of a Start button (§2.2): "Start team run for #123 Refunds on partial orders". */
export const startLabel = (item, mode) =>
  `Start ${mode === 'team' ? 'team run' : 'solo'} for ${item.ref} ${item.title}`;

// ---------------------------------------------------------------- time (notes §1.1)

/** "just now", "3 min ago", "2 h ago", "yesterday 18:02", "Mon 18:02". */
export function ageText(ms, now = Date.now()) {
  if (ms == null) return '';
  const m = Math.floor((now - ms) / 60000);
  if (m < 1) return 'just now';
  if (m < 60) return `${m} min ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} h ago`;
  const days = Math.round((startOfDay(now) - startOfDay(ms)) / 86400000);
  return days === 1 ? `yesterday ${clock(ms, ms)}` : clock(ms, now);
}

// ---------------------------------------------------------------- source errors (notes §4.2)

/** Where a `fix` of `integrations` lands: the Settings › Integrations row of the source. */
export const integrationsRow = (code) => (code?.startsWith('linear') ? 'linear' : 'github');

export const GH_SCOPE_COMMAND = 'gh auth refresh -s read:project';

/**
 * A source error in words (§4.2, F1–F3): { text, fix, tip, retry_at_ms }. `fix` is what the
 * control does: 'settings' | 'integrations' | 'gh_scope' (Copy command) | 'retry' | null (rate limit).
 * `error.message` is never shown but in the tooltip, clamped to 200 chars.
 */
export function errorInfo(error, source = {}, now = Date.now()) {
  const code = error?.code ?? 'failed';
  const tip = String(error?.message ?? '').slice(0, 200);
  const repo = source.label?.split(' · ')[0] ?? 'the repository';
  const one = (s) =>
    String(s ?? '')
      .split('\n')[0]
      .slice(0, 120);
  const t = (text, fix) => ({ code, text, fix, tip, retry_at_ms: error?.retry_at_ms ?? null });
  switch (code) {
    case 'gh_missing':
      return t('GitHub CLI not found', 'integrations');
    case 'gh_auth':
      return t('GitHub: not logged in', 'integrations');
    case 'gh_scope':
      return t('GitHub needs the read:project scope', 'gh_scope');
    case 'not_found':
      return t(`${repo} not found, or you have no access`, 'settings');
    case 'linear_key':
      return t('Linear: no API key', 'integrations');
    case 'linear_auth':
      return t('Linear rejected the API key', 'integrations');
    case 'rate_limited': {
      const at = error?.retry_at_ms;
      return t(
        `${source.type === 'linear' ? 'Linear' : 'GitHub'} rate limit reached${at ? ` · retry at ${clock(at, now)}` : ''}`,
        null,
      );
    }
    case 'bad_glob':
      return t("The glob doesn't parse", 'settings');
    case 'branch_missing':
      return t('Branch not found in this repo', 'settings');
    default:
      return t(`Could not refresh: ${one(error?.message)}`.trim(), 'retry');
  }
}

/** Whether Refresh is held by a rate limit (until its time). */
export const rateLimitedUntil = (source, now = Date.now()) =>
  source?.error?.code === 'rate_limited' && source.error.retry_at_ms > now ? source.error.retry_at_ms : null;

// ---------------------------------------------------------------- the dialogs (notes §5)

/** The origin chip (§5.1) of a prefill: { ref, source, url, text }. */
export function originChip(origin) {
  if (!origin) return null;
  if (origin.kind === 'brief')
    return { text: `From the Overwatch's brief · ${origin.title ?? 'brief'}`, brief: true, decision: origin.decision };
  const name = SOURCE_NAME[origin.source] ?? origin.source;
  return { text: `From ${origin.ref} · ${name}`, url: origin.url ?? null, source: origin.source, ref: origin.ref };
}

/** The `stale` error as the dialog's banner (§5.3). */
export function staleBanner(err) {
  const started = err?.started;
  if (started) return `Already started: ${started.name} (${started.kind === 'run' ? 'team run' : 'solo session'})`;
  if (err?.message === 'brief closed')
    return 'This brief was closed: it was dismissed, revised or its project was removed.';
  if (err?.message === 'not ready') return 'This item is hidden. Unhide it to start it.';
  return 'Already started.';
}

/** The toast when `intake_prefill` answers `stale` (§5.3). */
export function staleToast(err, ref = '') {
  const started = err?.started;
  if (started) return `${ref} was already started: ${started.name}.`.trim();
  if (err?.message === 'brief closed') return 'This brief was closed.';
  return `${ref} can't be started now.`.trim();
}

/** A stale error from a command (`{code: 'stale', …}` as an object or inside the error text). */
export function staleOf(e) {
  const o = e && typeof e === 'object' ? e : null;
  if (o?.code === 'stale') return o;
  try {
    const j = JSON.parse(String(e?.message ?? e));
    return j?.code === 'stale' ? j : null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- origin marks (notes §6)

/** The row chip: `from #123`, `from ENG-42`, `from checkout-v2.md`, `from brief`. */
export function originText(origin) {
  if (!origin) return '';
  if (origin.kind === 'brief') return 'from brief';
  return `from ${origin.short ?? origin.ref}`;
}

export function originTip(origin) {
  if (!origin) return '';
  if (origin.kind === 'brief') return `From the Overwatch's brief${origin.title ? `: ${origin.title}` : ''}`;
  return origin.ref ? `${origin.ref}${origin.url ? ` · ${origin.url}` : ''}` : '';
}

/** The details line / Inspector value (§6): "#123 Refunds on partial orders · GitHub ↗". */
export function originLine(origin, title = null) {
  if (!origin) return '';
  if (origin.kind === 'brief') return `the Overwatch's brief${origin.title ? `: ${origin.title}` : ''}`;
  return `${origin.ref}${title ? ` ${title}` : ''} · ${SOURCE_NAME[origin.source] ?? origin.source}`;
}

/** Markdown without links or images (notes §8.3): `[t](u)` → `t (u)`, `![alt](u)` → `alt`. */
export const plainMarkdown = (t) =>
  String(t ?? '')
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\(([^)]*)\)/g, '$1 ($2)');

// ---------------------------------------------------------------- the report line (notes §9)

/** "Started from intake: 9 of 10 · 7 items, 2 briefs · 1 not from intake"; '' when absent. */
export function reportLine(intake) {
  if (!intake) return '';
  const { started = 0, planned = 0, untracked = 0 } = intake;
  const total = started + planned + untracked;
  if (!total) return 'Nothing started this week';
  const parts = [`Started from intake: ${started + planned} of ${total}`];
  if (planned > 0) parts.push(`${plural(started, 'item')}, ${plural(planned, 'brief')}`);
  if (untracked > 0) parts.push(`${untracked} not from intake`);
  return parts.join(' · ');
}

// ---------------------------------------------------------------- the brief card (notes §8)

/** A brief that ended by a start or a dismissal (the planning tag clears; a revision keeps it). */
export const briefEnded = (d, since = 0) => {
  const lc = d?.lifecycle ?? {};
  const ended = lc.state === 'dismissed' || (lc.state === 'effect' && String(lc.effect ?? '').startsWith('launched'));
  return d?.kind === 'brief' && ended && (lc.effect_at_ms ?? lc.decided_at_ms ?? 0) >= since;
};

/** The report line's tooltip (F18): "Not from intake: agent fix-ci-3, run checkout"; '' without names. */
export const reportTip = (intake) =>
  intake?.untracked_items?.length
    ? `Not from intake: ${intake.untracked_items.map((x) => `${x.kind} ${x.ref}`).join(', ')}`
    : '';
