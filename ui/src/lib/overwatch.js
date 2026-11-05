// The Overwatch in the chat column (P4.4 spec §3.9, docs/design/p4-4-design-notes.md
// §1–§4, api-contract §3): its status in the header, the input's hints and the
// delivery markers of the user's messages, the deciding countdown, source links
// and annotations. Pure: the column renders what these return.

import { clock, plural } from './format.js';
import { cut } from './inbox.js';

/** What `now.overwatch` holds before the core says (an older core): the Overwatch is off. */
export const OFF = Object.freeze({ status: 'off', events: 0, since_ms: 0, agent: 'overwatch', has_session: false });
/** The longest message `chat_send` takes (ux F18) and where the counter starts. */
export const MAX_CHARS = 2000;
export const COUNT_FROM = 1800;
/** The Overwatch may settle a permission this long (spec §3.4); the ring drains over it. */
export const DECIDING_MS = 60000;
/** A long Overwatch answer clamps here (notes §2.1). */
export const CLAMP_LINES = 12;

/** `now.overwatch`, else the off record. */
export const overwatchOf = (now) => now?.overwatch ?? OFF;

// ---------------------------------------------------------------- status

const SHORT = { off: 'off', asleep: 'asleep', waking: 'waking…', paused: 'paused', down: 'down' };

/** The header's status (notes §1): "working · 2 events", "waking…". */
export function statusText(ow) {
  if (ow.status === 'working') return ow.events ? `working · ${plural(ow.events, 'event')}` : 'working';
  return SHORT[ow.status] ?? ow.status;
}

/** The status' tooltip and accessible name (notes §1). */
export function statusTip(ow) {
  switch (ow.status) {
    case 'off':
      return 'Overwatch is off. Turn it on in Settings.';
    case 'asleep':
      return 'Overwatch is asleep. It wakes on the next event.';
    case 'waking':
      return ow.events ? `Overwatch is waking · ${plural(ow.events, 'event')} waiting` : 'Overwatch is waking';
    case 'working':
      return ow.events ? `Overwatch is working on ${plural(ow.events, 'event')}` : 'Overwatch is working';
    case 'paused':
      return `Overwatch is paused${ow.why ? `: ${ow.why}` : ''}`;
    case 'down':
      return `Overwatch is down${ow.why ? `: ${ow.why}` : ''}. Cards still work.`;
    default:
      return `Overwatch · ${ow.status}`;
  }
}

/** The status opens the Overwatch's session when there is one today; `off` never does (notes §1). */
export const canOpenSession = (ow) => !!ow.has_session && ow.status !== 'off';

/** The strip under the header, only while down (notes §1). */
export const stripText = (ow) =>
  ow.status === 'down' ? `Overwatch is down${ow.why ? ` · ${ow.why}` : ''} · cards still work` : null;

/** The hint under the input for the status (notes §2): none while it is up. */
export function inputHint(ow) {
  switch (ow.status) {
    case 'paused':
      return 'Paused · it answers when it resumes';
    case 'down':
      return 'Down · messages are kept until it is back';
    case 'off':
      return 'Off · turn it on in Settings';
    default:
      return '';
  }
}

/** The marker under a user message that has not reached the Overwatch yet (notes §2, §8); '' once delivered. */
export function deliveryText(entry, ow) {
  if (entry.delivery === 'pending') {
    switch (ow.status) {
      case 'down':
        return 'Sent when the Overwatch is back';
      case 'paused':
        return 'Sent when the Overwatch resumes';
      case 'off':
        return 'Sent when the Overwatch is turned on';
      default:
        return 'waking…'; // it is back: the core delivers it now
    }
  }
  if (entry.delivery === 'waking' && (ow.status === 'asleep' || ow.status === 'waking')) return 'waking…';
  return '';
}

/** The status moves to `down` or comes back: one polite announcement (notes §1, §8); null otherwise. */
export function statusChange(prev, ow) {
  if (!prev || prev.status === ow.status) return null;
  if (ow.status === 'down') return `Overwatch is down${ow.why ? `: ${ow.why}` : ''}. Cards still work.`;
  if (prev.status === 'down' && ow.status !== 'off') return 'Overwatch is back.'; // switched off is not back
  return null;
}

// ---------------------------------------------------------------- deciding

/** A permission the Overwatch may still settle: its card shows "deciding" (notes §3). */
export const isDeciding = (d, now = Date.now()) =>
  d?.waits_on === 'overwatch' && d.deciding_until_ms != null && d.deciding_until_ms > now;

/** Whole seconds left of the deciding window. */
export const decidingLeft = (d, now = Date.now()) => Math.max(0, Math.ceil((d.deciding_until_ms - now) / 1000));

/** "Overwatch is deciding · 42 s left". */
export const decidingText = (d, now = Date.now()) => `Overwatch is deciding · ${decidingLeft(d, now)} s left`;

/** 0..1 of the ring still full. */
export const decidingShare = (d, now = Date.now()) =>
  Math.min(1, Math.max(0, (d.deciding_until_ms - now) / DECIDING_MS));

// ---------------------------------------------------------------- annotation

/** The action the Overwatch recommends, when the card has it (a stale id highlights nothing). */
export const recommended = (d) => {
  const id = d?.annotation?.recommend;
  return id && d.actions?.some((a) => a.id === id) ? id : null;
};

/** "recommends deny": the card's own label for the action, lowercased (notes §4.1). */
export function recommendsText(d) {
  const id = recommended(d);
  const label = d?.actions?.find((a) => a.id === id)?.label;
  return label ? `recommends ${label.toLowerCase()}` : '';
}

/** The note shows 3 lines, then "Show more" (notes §4.1, F8): about 3 × 45 characters at 316 px. */
export const NOTE_CLAMP = 140;
export const SOURCES_SHOWN = 3;

// ---------------------------------------------------------------- source links

const DOC = /^(?:\.\/)?[\w@.-]+(?:\/[\w@.-]+)*\.md(?:#.*)?$/i;

/**
 * What a link in the Overwatch's text or a source means (api-contract §5):
 * `chat:<id>` · `agent:<id>` · `run:<id>` · a doc path with an optional `#section`.
 * Anything else (http, mailto, unknown schemes) is null: the markdown renderer's own.
 */
export function refKind(url) {
  const u = String(url ?? '').trim();
  const m = /^(chat|agent|run):(.+)$/i.exec(u);
  if (m) return { kind: m[1].toLowerCase(), id: m[2] };
  if (!DOC.test(u)) return null;
  const [path, hash = ''] = u.split('#');
  const clean = path.replace(/^(\.\/)+/, '');
  return { kind: 'doc', path: clean, name: clean.split('/').at(-1), section: safeDecode(hash) || null };
}

function safeDecode(s) {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/** Where a feature doc lives: `.orchestrator/features/<slug>/<name>`. */
export function featureDoc(path) {
  const m = /(?:^|\/)\.orchestrator\/features\/([^/]+)\/(.+)$/.exec(path);
  return m ? { slug: m[1], name: m[2] } : null;
}

/** "api-contract.md §3" (`#3`), "spec.md#scope" (`#slug`). */
export function docLabel(ref) {
  if (!ref.section) return ref.name;
  return /^\d+(\.\d+)*$/.test(ref.section) ? `${ref.name} §${ref.section}` : `${ref.name}#${ref.section}`;
}

/**
 * A source's label (notes §2.1): a doc as `api-contract.md §3`, `chat:<id>` as
 * "your answer 11:02" / "your message 11:02" / "earlier card 11:02", `agent:`
 * as the agent, `run:` as "run <id>". `entries` = today's chat entries by id.
 */
export function sourceLabel(src, entries = {}, now = Date.now()) {
  const r = refKind(src);
  if (!r) return String(src);
  if (r.kind === 'doc') return docLabel(r);
  if (r.kind === 'agent') return r.id;
  if (r.kind === 'run') return `run ${r.id.replace(/^run-/, '')}`;
  const e = entries[r.id];
  if (!e) return null; // no longer available
  const at = clock(e.line?.at_ms ?? e.at_ms, now);
  if (e.kind === 'text') return `${e.author === 'user' ? 'your message' : 'earlier reply'} ${at}`;
  return `${e.line?.by === 'you' ? 'your answer' : 'earlier card'} ${at}`;
}

// ---------------------------------------------------------------- thread

/** "↳ re: why is checkout slow today?" when the Overwatch answers a user entry that is not just above it. */
export function replyRef(entry, prevKey, entries = {}) {
  const to = entry.reply_to;
  if (!to || to === prevKey) return null;
  const e = entries[to];
  return e ? { id: to, text: cut(String(e.text ?? '').replace(/\s+/g, ' '), 48) } : null;
}

/** The newest user entry with no answer yet (the one whose reply the column waits for), else null. */
export function awaiting(texts) {
  const user = texts.findLast((e) => e.author === 'user');
  if (!user || user.delivery === 'pending') return null;
  const answered = texts.some(
    (e) => e.author === 'overwatch' && e.at_ms >= user.at_ms && (!e.reply_to || e.reply_to === user.id),
  );
  return answered ? null : user;
}

/** "Overwatch is working on it · 12 s": elapsed since the message, to the 5 s. */
export function workingText(entry, now = Date.now()) {
  const s = Math.max(0, Math.floor((now - entry.at_ms) / 5000) * 5);
  return `Overwatch is working on it · ${s < 60 ? `${s} s` : `${Math.floor(s / 60)} min`}`;
}

/** The turn ended and nothing came back: the Overwatch went to sleep after the message (notes §2.2). */
export const noReply = (entry, ow) => !!entry && ow.status === 'asleep' && (ow.since_ms ?? 0) > entry.at_ms;

/** Over this many characters or lines an Overwatch entry clamps (notes §2.1). */
export const longText = (text) =>
  String(text ?? '').split('\n').length > CLAMP_LINES || String(text ?? '').length > 600;
