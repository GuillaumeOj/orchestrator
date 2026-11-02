// The chat column's thread (FR-1304/1305, P4.3 spec §3.3, docs/design/p4-3-design-notes.md):
// today's chat entries (api-contract §1) laid out in time order as the morning
// line, cards and collapsed lines, with the level filter and the fold. Pure:
// the column renders what `thread()` returns.

import { clock } from './format.js';
import { agentOf, cut, FILTERS } from './inbox.js';

/** A user's own decision collapses to its line this long after its effect (notes §2.1). */
export const COLLAPSE_MS = 10000;
/** Open cards shown before the newer ones fold (notes §4, F2). */
export const FOLD_AT = 5;

// Lifecycle states that keep a decision a card: still waiting, in flight, or
// needing a Dismiss (notes §2.1).
const CARD_STATES = new Set(['open', 'parked', 'decided', 'sent', 'failed', 'stale']);

// ---------------------------------------------------------------- entries

/** Upserts pushed entries into today's thread (api-contract §2): the highest
 *  `rev` wins; an entry of a later day starts the thread over; an earlier
 *  day's is not today's. Mutates and returns `chat`. */
export function upsertEntries(chat, list) {
  for (const e of list ?? []) {
    if (!e?.id) continue;
    if (chat.day && e.day && e.day < chat.day) continue;
    if (e.day && e.day !== chat.day) {
      chat.day = e.day;
      chat.entries = {};
    }
    const prev = chat.entries[e.id];
    if (prev && (prev.rev ?? 0) >= (e.rev ?? 0)) continue;
    chat.entries[e.id] = e;
  }
  return chat;
}

/** `now_snapshot.chat` as the column holds it. */
export function chatOf(snapshot) {
  return upsertEntries({ day: snapshot?.day ?? null, entries: {} }, snapshot?.entries);
}

const byTime = (x, y) => x.at_ms - y.at_ms || (x.id < y.id ? -1 : x.id > y.id ? 1 : 0);

// ---------------------------------------------------------------- lines

const WARN = new Set(['denied', 'rejected', 'failed', 'interrupted', 'expired']);
/** The verb's text: `run now` reads "started" (notes §2.2). */
export const verbText = (verb) => (verb === 'run now' ? 'started' : (verb ?? ''));
/** The verb takes the warning colour (the word is always there). */
export const warnVerb = (verb) => WARN.has(verb);

/** The line's tail from `by` (notes §2.2): none for you. */
export function tailText(line) {
  switch (line?.by) {
    case 'rule':
      return line.by_detail ? `rule: ${line.by_detail}` : 'rule';
    case 'pane':
      return 'in the pane';
    case 'expired':
      return line.verb === 'expired' ? '' : 'expired';
    case 'core':
      return 'by the app';
    case 'overwatch':
      return line.by_detail ? `Overwatch: ${line.by_detail}` : 'Overwatch';
    default:
      return '';
  }
}

/** "by rule installs", "in the pane" (notes §2.4, §8.2): who settled it, in words. */
export function byWords(line) {
  switch (line?.by) {
    case 'you':
      return 'by you';
    case 'rule':
      return line.by_detail ? `by rule ${line.by_detail}` : 'by rule';
    case 'pane':
      return 'in the pane';
    case 'expired':
      return line.verb === 'expired' ? '' : '(expired)';
    case 'core':
      return 'by the app';
    case 'overwatch':
      return 'by the Overwatch';
    default:
      return '';
  }
}

/** "allowed back-1: cargo add serde_json · rule: installs" (no time, no ▸). */
export function lineText(line) {
  const head = line.agent ? `${verbText(line.verb)} ${line.agent}: ` : `${verbText(line.verb)}: `;
  const tail = tailText(line);
  return `${head}${line.what ?? ''}${tail ? ` · ${tail}` : ''}`;
}

const cap = (s) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** The line's accessible name: "Allowed back-1: cargo add serde_json, by rule installs, 14:02". */
export function lineLabel(line, now = Date.now()) {
  const head = line.agent ? `${cap(verbText(line.verb))} ${line.agent}: ` : `${cap(verbText(line.verb))}: `;
  return [`${head}${line.what ?? ''}`, byWords(line), clock(line.at_ms, now)].filter(Boolean).join(', ');
}

/** The expanded line's marker: "Allowed by rule installs · 14:02". */
export function lineMarker(line, now = Date.now()) {
  return [`${cap(verbText(line.verb))}${byWords(line) ? ` ${byWords(line)}` : ''}`, clock(line.at_ms, now)].join(' · ');
}

/** "Already allowed in the pane · 14:02" (notes §8.2). */
export function alreadyText(line, now = Date.now()) {
  const by = byWords(line);
  return `Already ${verbText(line.verb)}${by ? ` ${by}` : ''} · ${clock(line.at_ms, now)}`;
}

/** The verb a closed decision's lifecycle says, for a decision the core has
 *  not posted a line for yet (the entry's `line` is the source). */
function verbOf(d) {
  const lc = d.lifecycle ?? {};
  if (lc.state === 'dismissed') return lc.effect === 'restarted' ? 'restarted' : 'dismissed';
  if (lc.state === 'interrupted') return 'interrupted';
  if (lc.state === 'failed') return 'failed';
  switch (lc.effect) {
    case 'allowed':
      return 'allowed';
    case 'denied':
      return 'denied';
    case 'pr_opened':
      return 'approved';
    case 'restarted':
      return 'restarted';
    case 'applied':
      return d.kind === 'ask' ? 'run now' : 'applied';
    case 'skipped':
      return d.kind === 'ask' ? 'kept queued' : 'skipped';
    case 'deleted':
    case 'dismissed':
      return 'dismissed';
    case 'queued':
      return d.kind === 'ask' ? 'kept queued' : 'answered';
    default:
      if (d.kind === 'permission') return 'allowed';
      if (d.kind === 'gate') return 'approved';
      if (d.kind === 'ask') return 'run now';
      return 'answered';
  }
}

const BY = { user: 'you', pane: 'pane', expired: 'expired', core: 'core', overwatch: 'overwatch' };

/** A line composed from the decision itself (api-contract §1 shape). */
export function lineOfDecision(d, now = Date.now()) {
  const lc = d.lifecycle ?? {};
  const by = lc.decided_by ?? '';
  return {
    verb: verbOf(d),
    agent: agentOf(d) ?? undefined,
    what: cut(d.title, 48),
    by: by.startsWith('rule:') ? 'rule' : (BY[by] ?? 'core'),
    by_detail: by.startsWith('rule:') ? by.slice(5) : undefined,
    state: lc.state,
    at_ms: lc.effect_at_ms ?? lc.decided_at_ms ?? now,
  };
}

// ---------------------------------------------------------------- thread

// The line composed for a decision with no `line` yet, per decision object (a
// push replaces the object): the same line each call, so the memo keeps its item.
const composed = new WeakMap();
function composedLine(d, now) {
  let l = composed.get(d);
  if (!l) composed.set(d, (l = lineOfDecision(d, now)));
  return l;
}

/**
 * @typedef {{ type: 'morning' | 'card' | 'line' | 'fold', key: string, id?: string, d?: any,
 *   entry?: any, line?: any, ref?: any, n?: number, blocking?: number, open?: boolean }} Item
 */

/**
 * The thread to render. `chat` = { day, entries }; `decisions` = now.decisions;
 * `pending` = app.pending; `seen` = Map decision id → when the column first saw
 * its effect (the collapse clock when the lifecycle has no effect_at_ms);
 * `held` = ids whose collapse waits (pointer over it, a reply typed); `memo` =
 * the previous call's items by key, reused when unchanged so a `now` event
 * re-renders only what it touched (spec §5: < 16 ms for 300 entries).
 *
 * Returns { items, cards, all, counts, open, shown, nextDue }:
 * items   [{type:'morning'|'card'|'line'|'fold', key, …}] in order
 * cards   the card items shown (j/k stops), in order
 * all     every card item, before the filter and the fold
 * counts  open cards per filter id, before filtering (the chips)
 * nextDue the next time a card collapses (null: none)
 */
export function thread({
  chat,
  decisions = {},
  pending = [],
  filter = 'all',
  foldOpen = false,
  seen = new Map(),
  held = new Set(),
  now = Date.now(),
  memo = null,
}) {
  const entries = Object.values(chat?.entries ?? {});
  const waiting = new Set(pending.filter((p) => !p.sent).map((p) => p.item));
  // The latest entry of each decision (a returning synthesized id has a newer one).
  const latest = new Map();
  for (const e of entries)
    if (e.kind === 'decision' && e.decision) {
      const l = latest.get(e.decision);
      if (!l || byTime(l, e) < 0) latest.set(e.decision, e);
    }
  // A decision listed with no entry yet (an older core, a push in flight) sits
  // at its creation time.
  for (const d of Object.values(decisions))
    if (!latest.has(d.id) && d.level < 4) {
      const e = { id: `d:${d.id}`, kind: 'decision', decision: d.id, thread: d.thread, at_ms: d.created_ms ?? now };
      entries.push(e);
      latest.set(d.id, e);
    }
  entries.sort(byTime);

  let nextDue = null;
  /** 'card' | 'line' | null for a decision entry. */
  const mode = (e, d) => {
    if (d && latest.get(d.id) === e) {
      if (d.level >= 4) return e.line ? 'line' : null;
      const lc = d.lifecycle ?? {};
      if (waiting.has(d.id) || CARD_STATES.has(lc.state)) return 'card';
      if (lc.state === 'effect' && lc.decided_by === 'user') {
        const due = (lc.effect_at_ms ?? seen.get(d.id) ?? now) + COLLAPSE_MS;
        if (held.has(d.id) || now < due) {
          if (!held.has(d.id)) nextDue = nextDue == null ? due : Math.min(nextDue, due);
          return 'card';
        }
      }
      return 'line';
    }
    return e.line ? 'line' : null;
  };

  /** @type {Item[]} */
  const all = [];
  for (const e of entries) {
    if (e.kind === 'morning') all.push({ type: 'morning', key: e.id, entry: e });
    if (e.kind !== 'decision') continue;
    const d = decisions[e.decision] ?? null;
    const m = mode(e, d);
    if (m === 'card') all.push({ type: 'card', key: e.id, id: d.id, d, entry: e });
    else if (m === 'line') {
      const live = d && latest.get(d.id) === e;
      const line = e.line ?? composedLine(d, now);
      all.push({ type: 'line', key: e.id, id: e.decision, line, d: (live ? d : null) ?? e.card ?? d, entry: e });
    }
  }
  const reuse = (it) => {
    const old = memo?.get(it.key);
    return old && sameItem(old, it) ? old : it;
  };
  for (let i = 0; i < all.length; i++) if (all[i].type !== 'card') all[i] = reuse(all[i]);

  // "↳ after": the latest earlier decision of the same thread that closed today (notes §3).
  const closed = new Map();
  for (const it of all)
    if (it.type === 'line' && it.entry.thread) {
      const list = closed.get(it.entry.thread) ?? [];
      list.push(it);
      closed.set(it.entry.thread, list);
    }
  for (const it of all) {
    if (it.type !== 'card' || !it.entry.thread) continue;
    let best = null;
    for (const l of closed.get(it.entry.thread) ?? [])
      if (l.line.at_ms < it.entry.at_ms && (!best || l.line.at_ms > best.line.at_ms)) best = l;
    if (best) it.ref = best;
  }
  for (let i = 0; i < all.length; i++) if (all[i].type === 'card') all[i] = reuse(all[i]);
  if (memo) {
    memo.clear();
    for (const it of all) memo.set(it.key, it);
  }

  const cardsAll = all.filter((i) => i.type === 'card');
  const counts = Object.fromEntries(FILTERS.map((f) => [f.id, cardsAll.filter((i) => f.test(i.d)).length]));
  const test = (FILTERS.find((f) => f.id === filter) ?? FILTERS[0]).test;
  const shown = cardsAll.filter((i) => test(i.d));
  const folded = new Set(shown.slice(FOLD_AT).map((i) => i.key));

  /** @type {Item[]} */
  const items = [];
  /** @type {Item[]} */
  const cards = [];
  for (const it of all) {
    if (it.type === 'card') {
      if (!test(it.d)) continue;
      if (folded.size && it.key === shown[FOLD_AT].key) {
        const rest = shown.slice(FOLD_AT);
        items.push({
          type: 'fold',
          key: 'fold',
          n: rest.length,
          blocking: rest.filter((i) => i.d.level === 1).length,
          open: foldOpen,
        });
      }
      if (folded.has(it.key) && !foldOpen) continue;
      cards.push(it);
    }
    items.push(it);
  }
  return { items, cards, all: cardsAll, counts, open: cardsAll.length, shown: shown.length, nextDue };
}

/** The same item as last time: what it renders from did not change. */
function sameItem(a, b) {
  return (
    a.type === b.type &&
    a.id === b.id &&
    a.d === b.d &&
    a.line === b.line &&
    a.ref === b.ref &&
    (a.entry === b.entry ||
      (a.entry.id.startsWith('d:') && a.entry.at_ms === b.entry.at_ms && a.entry.thread === b.entry.thread))
  );
}

/** "3 more open · 1 blocking · j" / "Fold 3 cards · ⏎" (notes §4). */
export function foldText(f) {
  if (f.open) return `Fold ${f.n} cards`;
  return `${f.n} more open${f.blocking ? ` · ${f.blocking} blocking` : ''}`;
}

/** The filter strip (notes §5): null when nothing is hidden. */
export function filterStrip(filter, counts) {
  if (filter === 'all') return null;
  const label = FILTERS.find((f) => f.id === filter)?.label ?? filter;
  const shown = counts[filter] ?? 0;
  const hidden = counts.all - shown;
  if (!hidden) return null;
  if (!shown) return `No ${label} cards open. ${counts.all} open in other filters`;
  return `Showing ${label} · ${shown} of ${counts.all} open cards. ${hidden} hidden`;
}

/** The overlay's banner (notes §9): null when no card is open. */
export function bannerText(open, blocking) {
  if (!open) return null;
  return `${open} card${open === 1 ? '' : 's'} need${open === 1 ? 's' : ''} you${blocking ? `, ${blocking} blocking` : ''}`;
}

/** The title bar badge's label (notes §11.1). */
export const badgeLabel = (n) =>
  n ? `${n} decision${n === 1 ? '' : 's'} need${n === 1 ? 's' : ''} you · ⌘⇧I` : 'No decisions waiting · ⌘⇧I';

/** The digest group's header and body (notes §7). */
export function digestText(outcomes, doneToday) {
  const n = outcomes.length;
  if (n) {
    const head = `Today · ${n} outcome${n === 1 ? '' : 's'}`;
    return { head, clear: true, more: doneToday > n ? `${doneToday} done today` : null, body: null };
  }
  if (doneToday) return { head: `Today · ${doneToday} done`, clear: false, body: `${doneToday} done today, all seen.` };
  return { head: 'Today · no outcomes yet', clear: false, body: 'Nothing done yet today.' };
}

/** Next filter for `f` (notes §5): All → Blocking → Errors → Budget, skipping empty chips. */
export function nextFilter(filter, counts) {
  const ids = FILTERS.map((f) => f.id);
  for (let i = 1; i <= ids.length; i++) {
    const id = ids[(ids.indexOf(filter) + i) % ids.length];
    if (id === 'all' || counts[id]) return id;
  }
  return 'all';
}
