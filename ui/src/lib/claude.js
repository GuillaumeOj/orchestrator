// Updating Claude Code from the app (add-update-claude-code ux-brief, api-contract §1):
// Settings › Claude Code, the status bar chip, the nav badge, the read-only banner
// clause and the toasts, all from `app_state.setup.claude` (`c` below; null until the
// start-up check lands). Pure: tested in test/claude.test.js.
import { clock } from './format.js';

/** "2.1.330" → [2, 1, 330]; null when it has no digits. */
const parts = (v) => {
  const m = /\d+(?:\.\d+)*/.exec(String(v ?? ''));
  return m ? m[0].split('.').map(Number) : null;
};

/** -1 | 0 | 1, or null when either side is unknown. */
export function cmpVersion(a, b) {
  const x = parts(a);
  const y = parts(b);
  if (!x || !y) return null;
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d) return d < 0 ? -1 : 1;
  }
  return 0;
}

const INSTALLS = { native: 'native installer', homebrew: 'Homebrew', npm: 'npm', unknown: 'unknown install' };
export const installName = (i) => INSTALLS[i] ?? INSTALLS.unknown;

/** 0:42, 12:05, 1:02:03 (tabular in the UI). */
export function elapsed(startedAt, now = Date.now()) {
  if (!startedAt) return '0:00';
  const s = Math.max(0, Math.floor(now / 1000 - startedAt));
  const sec = String(s % 60).padStart(2, '0');
  const m = Math.floor(s / 60);
  return m < 60 ? `${m}:${sec}` : `${Math.floor(m / 60)}:${String(m % 60).padStart(2, '0')}:${sec}`;
}

const when = (secs) => (secs ? clock(secs * 1000) : null);

/**
 * The update block (ux-brief §2.1.2), first match wins:
 * not found › updating › update failed › check failed › checking › updated › available › up to date › never checked.
 * Returns `{ kind, lead, rest?, tone, glyph, line2?, before?, buttons, log, logLabel?, tip? }`:
 * - `tone`: muted | red | amber | green | accent | running (dot colour and text tone);
 * - `glyph`: dot | spinner | ring | warn;
 * - `buttons`: `{ id: 'update' | 'check', label, primary?, def? }`, right-aligned, primary last;
 * - `log`: 'open' (updating / failed) | 'collapsible' (updated) | null.
 */
export function updateBlock(c, now = Date.now()) {
  if (!c)
    return { kind: 'pending', lead: 'Checking Claude Code…', tone: 'muted', glyph: 'spinner', buttons: [], log: null };
  const u = c.update ?? {};
  const check = { id: 'check', label: 'Check now' };
  const upd = (label, extra = {}) => ({ id: 'update', label, ...extra });
  if (c.problem_kind === 'not_found')
    return { kind: 'not-found', lead: c.problem, tone: 'red', glyph: 'dot', buttons: [], log: null, setup: true };
  if (u.state === 'updating')
    return {
      kind: 'updating',
      lead: u.latest ? `Updating to ${u.latest}…` : 'Updating Claude Code…',
      rest: elapsed(u.started_at, now),
      tone: 'running',
      glyph: 'spinner',
      buttons: [],
      log: 'open',
    };
  if (u.state === 'error' && u.error_from === 'update')
    return {
      kind: 'update-failed',
      lead: `Update failed: ${u.error ?? 'unknown error'}`,
      tone: 'amber',
      glyph: 'warn',
      tip: u.error,
      buttons: [{ ...upd('Retry'), def: true }, check],
      log: 'open',
      logLabel: 'Output of claude update',
    };
  if (u.state === 'error')
    return {
      kind: 'check-failed',
      lead: `Couldn't check for updates: ${u.error ?? 'unknown error'}`,
      tone: 'amber',
      glyph: 'warn',
      tip: u.error,
      line2:
        u.latest && u.available
          ? `Last known: ${u.latest}${when(u.checked_at) ? ` (checked ${when(u.checked_at)})` : ''}`
          : null,
      buttons: [{ ...check, def: true }, upd('Update anyway')],
      log: null,
    };
  if (u.state === 'checking')
    return { kind: 'checking', lead: 'Checking for updates…', tone: 'muted', glyph: 'spinner', buttons: [], log: null };
  const updated = u.updated ? `Updated ${u.updated.from} → ${u.updated.to}.` : null;
  if (u.available)
    return {
      kind: 'available',
      lead: `${u.latest} is available`,
      tone: 'accent',
      glyph: 'dot',
      before: updated,
      buttons: [check, { ...upd(`Update to ${u.latest}`), primary: true, def: true }],
      log: null,
    };
  if (updated)
    return {
      kind: 'updated',
      lead: updated,
      tone: 'green',
      glyph: 'dot',
      line2:
        'New and resumed agents use it; agents already running keep the version they started with until they restart.',
      buttons: [{ ...check, def: true }],
      log: u.log?.length ? 'collapsible' : null,
    };
  if (u.latest)
    return {
      kind: 'up-to-date',
      lead: 'Up to date',
      rest: when(u.checked_at) ? ` · checked ${when(u.checked_at)}` : '',
      tone: 'green',
      glyph: 'dot',
      buttons: [{ ...check, def: true }],
      log: null,
    };
  return {
    kind: 'never',
    lead: 'Not checked yet',
    tone: 'muted',
    glyph: 'ring',
    buttons: [{ ...check, def: true }],
    log: null,
  };
}

/** One muted line under the update block, or null (ux-brief §2.1.4). */
export function hint(c) {
  if (!c || c.problem_kind === 'not_found') return null;
  if (c.install === 'homebrew') return 'Homebrew installs update through brew; this may take a while.';
  if (c.install === 'unknown') return 'If the update fails, update Claude Code the way you installed it.';
  return null;
}

/** The header block's lines: version, install, channel, path (ux-brief §2.1.1). */
export function installedHeader(c) {
  if (!c) return { version: '…', meta: '', path: null };
  if (c.problem_kind === 'not_found')
    return { version: '—', meta: '', path: null, missing: 'claude not found on PATH' };
  return {
    version: c.version ?? '—',
    meta: [installName(c.install), c.channel ? `channel ${c.channel}` : null].filter(Boolean).join(' · '),
    path: c.path ?? null,
  };
}

/** Compatibility rows and notes: `{ supported, notes: [{ tone: 'amber' | 'red', text }] }` (ux-brief §2.1.5). */
export function compat(c) {
  const s = c?.supported;
  const notes = [];
  if (!s) return { supported: null, notes };
  const newer = 'Agents run, with a warning.';
  if (c.problem_kind === 'version' || cmpVersion(c.version, s.min) === -1)
    notes.push({ tone: 'red', text: 'Too old. The app is read-only until Claude Code is updated.' });
  if (cmpVersion(c.version, s.tested_max) === 1)
    notes.push({ tone: 'amber', text: `Newer than the last tested version. ${newer}` });
  const latest = c.update?.latest;
  if (c.update?.available && cmpVersion(latest, s.tested_max) === 1)
    notes.push({ tone: 'amber', text: `${latest} is newer than the last tested version. ${newer}` });
  return { supported: `${s.min} (minimum) – ${s.tested_max} (last tested)`, notes };
}

/** The Account rows: `{ login, note }`. */
export function account(c) {
  if (!c || c.logged_in == null) return null;
  const login = c.logged_in
    ? ['Logged in', c.auth_method, c.subscription].filter(Boolean).join(' · ')
    : 'Not logged in';
  return { login, note: c.note ?? null };
}

/** The keys that are live now (hints line and handler): `u` runs update, `r` checks. */
export function liveKeys(c, now = Date.now()) {
  const b = updateBlock(c, now).buttons;
  return { u: b.some((x) => x.id === 'update'), r: b.some((x) => x.id === 'check') };
}

/** The Settings nav entry's badge: error dot › amber dot › ↑ › none; a spinner while updating. */
export function navBadge(c) {
  if (!c) return null;
  const u = c.update;
  if (u?.state === 'updating') return { kind: 'spinner', title: 'Updating Claude Code…' };
  if (c.problem) return { kind: 'error', title: c.problem };
  if (u?.state === 'error' && u.error_from === 'update')
    return { kind: 'warn', title: `Claude Code update failed: ${u.error}` };
  if (u?.available) return { kind: 'up', title: `Claude Code ${u.latest} is available` };
  return null;
}

/**
 * The status bar chip: updating › update failed › available › null. Silent for a failed check,
 * and while the read-only banner carries a too-old version. `kind`: updating | failed | available.
 */
export function claudeChip(c) {
  const u = c?.update;
  if (!u || c.problem_kind === 'version' || c.problem_kind === 'not_found') return null;
  if (u.state === 'updating')
    return { kind: 'updating', text: 'Updating Claude Code…', title: 'Running claude update. Click for the output.' };
  if (u.state === 'error' && u.error_from === 'update')
    return {
      kind: 'failed',
      text: '⚠ Claude Code update',
      title: `Claude Code update failed: ${u.error}. Click for details.`,
    };
  if (u.available)
    return {
      kind: 'available',
      text: '↑ Claude Code',
      version: u.latest,
      title: `Claude Code ${u.latest} is available (you run ${c.version}). Click to open Settings.`,
    };
  return null;
}

/** The toast for what changed between two `setup.claude` reads, or null (never at start-up). */
export function claudeToast(prev, next) {
  const p = prev?.update;
  const n = next?.update;
  if (!p || !n) return null;
  if (n.updated && n.updated.at !== p.updated?.at) return { text: `Claude Code updated to ${n.updated.to}.`, ms: 3000 };
  // A failed update reads as an error once; the same error on later reads is not news.
  if (n.state === 'error' && n.error_from === 'update' && (p.state !== 'error' || p.error_from !== 'update'))
    return { text: 'Claude Code update failed.', kind: 'warn', sticky: true, details: true };
  return null;
}

/** The toast for a palette-triggered check, once it settled. */
export function checkToast(c) {
  const u = c?.update;
  if (!u) return null;
  if (u.state === 'error')
    return { text: `Couldn't check for Claude Code updates: ${u.error ?? 'unknown error'}`, kind: 'error' };
  if (u.available) return { text: `Claude Code ${u.latest} is available.` };
  return { text: 'Claude Code is up to date.' };
}

/** Palette: `why` is the greyed-out hint, null when the command can run. */
export function paletteActions(c) {
  const busy = !c
    ? 'Claude Code is being checked'
    : c.problem_kind === 'not_found'
      ? 'Claude Code not found'
      : c.update?.state === 'updating'
        ? 'an update is running'
        : null;
  return {
    check: busy ?? (c.update?.state === 'checking' ? 'already checking' : null),
    update: busy ?? (c.update?.available ? null : 'no update available'),
  };
}

/**
 * The read-only banner's version clause (ux-brief §2.4), or null for other problems.
 * `fail` is the amber clause; `button` the update action; `details` replaces "Set up" after a failure.
 */
export function bannerClaude(c) {
  if (c?.problem_kind !== 'version') return null;
  const u = c.update ?? {};
  const failed = u.state === 'error' && u.error_from === 'update';
  const updating = u.state === 'updating';
  return {
    text: c.problem,
    fail: failed ? `Update failed: ${u.error ?? 'unknown error'}` : null,
    button: updating
      ? { label: 'Updating…', disabled: true, spinner: true }
      : { label: failed ? 'Retry update' : 'Update Claude Code' },
    details: failed,
    key: `${c.problem}|${failed ? u.error : ''}`,
  };
}
