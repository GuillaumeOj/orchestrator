// The Overwatch in the chat column (P4.4 spec §3.9, docs/design/p4-4-design-notes.md
// §1–§4, api-contract §3): its status in the header, the input's hints and the
// delivery markers of the user's messages, the deciding countdown, source links
// and annotations. Pure: the column renders what these return.

import { clock, plural } from './format.js';
import { cut } from './inbox.js';
import { safeDecode } from './markdown.js';

/** What `now.overwatch` holds before the core says (an older core): the Overwatch is off. */
export const OFF = Object.freeze({ status: 'off', events: 0, since_ms: 0, agent: 'overwatch', has_session: false });
/** The longest message `chat_send` takes (ux F18) and where the counter starts. */
export const MAX_CHARS = 2000;
export const COUNT_FROM = 1800;
/** The Overwatch may settle a permission this long (spec §3.4); the ring drains over it. */
const DECIDING_MS = 60000;

/** `now.overwatch`, else the off record. */
export const overwatchOf = (now) => now?.overwatch ?? OFF;

// ---------------------------------------------------------------- status

const SHORT = { off: 'off', asleep: 'asleep', waking: 'waking…', paused: 'paused', down: 'down' };

/** The header's status (notes §1): "working · 2 events", "waking…". */
export function statusText(ow) {
  if (ow.status === 'working') return ow.events ? `working · ${plural(ow.events, 'event')}` : 'working';
  return SHORT[ow.status] ?? ow.status;
}

/** The status' tooltip and accessible name (notes §1); `open`: it is a control, ⏎ opens its session. */
export function statusTip(ow, open = false) {
  const tip = baseTip(ow);
  return open ? `${tip} · ⏎ open its session` : tip;
}

/** "Overwatch is down: login expired. Cards still work." */
const downText = (ow) => `Overwatch is down${ow.why ? `: ${ow.why}` : ''}. Cards still work.`;

/** The settings section's status (notes §9.1): "Overwatch · down: login expired", null when it is up. */
export function sectionStatus(ow) {
  if (ow.status !== 'down' && ow.status !== 'paused') return null;
  return `Overwatch · ${ow.status}${ow.why ? `: ${ow.why}` : ''}`;
}

function baseTip(ow) {
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
      return downText(ow);
    default:
      return `Overwatch · ${ow.status}`;
  }
}

/** The status opens the Overwatch's session when there is one today; `off` never does (notes §1). */
export const canOpenSession = (ow) => !!ow.has_session && ow.status !== 'off';

/** The strip under the header, only while down: its two lines (notes §1). */
export const stripText = (ow) =>
  ow.status === 'down'
    ? { title: 'Overwatch is down', detail: `${ow.why ? `${ow.why} · ` : ''}cards still work` }
    : null;

/** The hint under the input for the status (notes §2): `{ text, link }`, `link` the words that open Settings. */
export function inputHint(ow) {
  switch (ow.status) {
    case 'paused':
      return { text: 'Paused · it answers when it resumes', link: null };
    case 'down':
      return { text: 'Down · messages are kept until it is back', link: null };
    case 'off':
      return { text: 'Off · ', link: 'turn it on in Settings' };
    default:
      return { text: '', link: null };
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
  if (ow.status === 'down') return downText(ow);
  // Only a real return: switched off or paused is not "back" (notes §8).
  if (prev.status === 'down' && ['asleep', 'waking', 'working'].includes(ow.status)) return 'Overwatch is back.';
  return null;
}

// ---------------------------------------------------------------- deciding

/** A permission the Overwatch may still settle: its card shows "deciding" (notes §3). */
export const isDeciding = (d, now = Date.now()) =>
  d?.waits_on === 'overwatch' && d.deciding_until_ms != null && d.deciding_until_ms > now;

/** Whole seconds left of the deciding window. */
const decidingLeft = (d, now = Date.now()) => Math.max(0, Math.ceil((d.deciding_until_ms - now) / 1000));

/** "Overwatch is deciding · 42 s left". */
export const decidingText = (d, now = Date.now()) => `Overwatch is deciding · ${decidingLeft(d, now)} s left`;

/** The whole window in ms: from the card's creation to the deadline (`permission_secs`), else a minute. */
const decidingWindow = (d) => {
  const w = d.deciding_until_ms - (d.created_ms ?? 0);
  return w > 0 && w <= 300000 ? w : DECIDING_MS;
};

/** 0..1 of the ring still full. */
export const decidingShare = (d, now = Date.now()) =>
  Math.min(1, Math.max(0, (d.deciding_until_ms - now) / decidingWindow(d)));

/** The marker's static accessible name: "Overwatch is deciding, up to 1 minute" / "up to 90 s". */
export function decidingName(d) {
  const s = Math.round(decidingWindow(d) / 1000);
  return `Overwatch is deciding, up to ${s % 60 === 0 ? plural(s / 60, 'minute') : `${s} s`}`;
}

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
  if (!DOC.test(u) || u.split('#')[0].split('/').includes('..')) return null;
  const [path, hash = ''] = u.split('#');
  const clean = path.replace(/^(\.\/)+/, '');
  return { kind: 'doc', path: clean, name: clean.split('/').at(-1), section: safeDecode(hash) || null };
}

/** Where a feature doc lives: `.orchestrator/features/<slug>/<name>`. */
export function featureDoc(path) {
  const m = /(?:^|\/)\.orchestrator\/features\/([^/]+)\/(.+)$/.exec(path);
  return m ? { slug: m[1], name: m[2] } : null;
}

/** "api-contract.md §3" (`#3`), "spec.md#scope" (`#slug`). */
function docLabel(ref) {
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
  return r ? refLabel(r, entries, now) : String(src);
}

/** The label of a parsed ref (null: no longer available). */
export function refLabel(r, entries = {}, now = Date.now()) {
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

// ---------------------------------------------------------------- undo on lines (notes §6)

/** Per contract label: the collapsed word, the busy button, the toast after the core confirms. */
const UNDOS = {
  'Revoke rule': { short: 'Undo', busy: 'Revoking' },
  'Tell the agent': { short: 'Undo', busy: 'Telling' },
  'Allow instead': { short: 'Undo', busy: 'Allowing' },
  'Correct the answer': { short: 'Correct', busy: 'Sending', input: true },
  'Restore tier': { short: 'Undo', busy: 'Restoring' },
  'Remove rule': { short: 'Undo', busy: 'Removing' },
};

/** Unsticking verbs: they ran, there is nothing to take back (notes §6.3). */
const ACTIONS = new Set(['nudged', 'resumed', 'restarted']);

/** The tier a reprioritized line moved from: "Normal → High in the queue" gives "Normal". */
const tierFrom = (line) => /^(\w+) → /.exec(line.what ?? '')?.[1] ?? null;

/**
 * Where a line's undo stands: `available` (a button), `done` (undone or corrected),
 * `gone` (could have been, the condition is gone) or `none` (nothing to undo).
 */
export function undoState(line) {
  if (line?.undone_at_ms || line?.corrected_at_ms) return 'done';
  if (line?.undo) return 'available';
  if (line?.undo_gone) return 'gone';
  return 'none';
}

/** The word an undone line shows after its tail (notes §6.2): "undone 14:03", "corrected 14:05". */
export const undoneTail = (line, now = Date.now()) =>
  line.corrected_at_ms
    ? `corrected ${clock(line.corrected_at_ms, now)}`
    : line.undone_at_ms
      ? `undone ${clock(line.undone_at_ms, now)}`
      : '';

/**
 * What the expanded line says about its undo (notes §6.1–6.3): the button's label and busy
 * label, the explanation under it, the toast after it, the marker once done, or the
 * "Can't be undone" line. `project` names the policy file's project when known.
 */
export function undoInfo(line, project = null, now = Date.now()) {
  const state = undoState(line);
  const agent = line.agent ?? 'the agent';
  const rule = line.by_detail ?? 'the rule';
  const file = project ? `${project} policy` : 'its policy file';
  const label = line.undo?.label ?? '';
  const u = UNDOS[label] ?? { short: 'Undo', busy: 'Undoing' };
  // A line already undone no longer carries `undo`: the verb and its source say what it was.
  const kind = label || guessLabel(line);
  const at = (ms) => clock(ms, now);
  const info = {
    state,
    label,
    short: u.short,
    busy: `${u.busy} ${label === 'Revoke rule' || label === 'Remove rule' ? 'the rule' : agent}…`,
    input: !!u.input,
    explain: '',
    toast: '',
    done: '',
    why: '',
  };
  switch (kind) {
    case 'Revoke rule':
      info.explain = `Removes rule ${rule} from ${file}. The call already ran.`;
      info.toast = `Rule ${rule} revoked.`;
      info.done = `Rule ${rule} revoked · ${at(line.undone_at_ms)}`;
      break;
    case 'Tell the agent':
      info.explain = `Tells ${agent} not to repeat this call and denies the next identical one for 1 hour. The call already ran.`;
      info.toast = `Told ${agent} not to repeat it.`;
      info.done = `Undone ${at(line.undone_at_ms)} · ${agent} was told not to repeat it`;
      break;
    case 'Allow instead':
      info.explain = `Lets ${agent} retry this exact call once and tells it so.`;
      info.toast = `${agent} may retry.`;
      info.done = `Undone ${at(line.undone_at_ms)} · ${agent} was told it may retry`;
      break;
    case 'Correct the answer':
      info.explain = `Send ${agent} what it should know instead.`;
      info.toast = `Told ${agent} the answer changed.`;
      info.done = `Corrected ${at(line.corrected_at_ms)} · ${agent} was told`;
      break;
    case 'Restore tier': {
      const from = tierFrom(line);
      info.explain = `Puts ${agent} back at ${from ?? 'its earlier tier'}.`;
      info.toast = `Restored ${agent} to ${from ?? 'its earlier tier'}.`;
      info.done = `Restored${from ? ` to ${from}` : ''} · ${at(line.undone_at_ms)}`;
      break;
    }
    case 'Remove rule':
      info.explain = `Removes the rule from ${file}.`;
      info.toast = 'Rule removed.';
      info.done = `Rule removed · ${at(line.undone_at_ms)}`;
      break;
  }
  if (state === 'gone') info.why = `Can't be undone: ${line.undo_gone}`;
  else if (state === 'none')
    info.why =
      line.by === 'core'
        ? 'Always denied by the app. Nothing can change it.'
        : ACTIONS.has(line.verb)
          ? "Can't be undone: it already ran."
          : '';
  return info;
}

/** The label an undone line had, from what it says (its `undo` is gone once used). */
function guessLabel(line) {
  if (line.corrected_at_ms) return 'Correct the answer';
  if (!line.undone_at_ms) return '';
  if (line.verb === 'reprioritized') return 'Restore tier';
  if (line.verb === 'added') return 'Remove rule';
  if (line.by === 'policy') return 'Revoke rule';
  if (line.verb === 'denied') return 'Allow instead';
  return 'Tell the agent';
}

/** The undo word on a collapsed line's right edge: "Undo u", "Correct u" (empty when none). */
export const undoChip = (line) => {
  if (undoState(line) !== 'available') return '';
  return `${UNDOS[line.undo.label]?.short ?? 'Undo'} u`;
};
