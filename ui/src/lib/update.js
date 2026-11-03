// The in-app updater in the UI (ux-brief, api-contract §3): the status bar chip,
// the toast and the palette copy, from the `update` object of `app_state`.
// Pure: tested in test/update.test.js.
import { clock } from './format.js';

const pct = (p) => Math.round(p * 100);

/** The status bar item, or null when there is nothing to say (idle, checking, disabled).
 *  `kind`: 'downloading' (plain text), 'ready' (button, accent border), 'error' (button, amber). */
export function updateChip(u) {
  if (!u?.enabled) return null;
  const v = u.available?.version;
  switch (u.state) {
    case 'downloading':
      return {
        kind: 'downloading',
        text: `↓ ${v}${u.progress != null ? ` · ${pct(u.progress)} %` : ''}`,
        title: `Downloading Orchestrator ${v}`,
      };
    case 'ready':
      return {
        kind: 'ready',
        text: `⟳ ${v} · Restart`,
        title: u.apply_on_quit
          ? 'Installs when you quit. Click to restart now (agents are suspended and resume).'
          : isNonMain(u)
            ? `Back to main ${v}: restarts into it (agents are suspended and resume).`
            : 'Your build is local: it is replaced only if you restart now.',
      };
    case 'error':
      return {
        kind: 'error',
        text: '⚠ update',
        title: `${u.error ?? 'Update failed'}${u.checked_at ? ` · last checked ${clock(u.checked_at * 1000)}` : ''}`,
      };
    default:
      return null;
  }
}

/** Running a PR or a local build (not `main`): main updates never apply on quit. */
export const isNonMain = (u) => u?.current?.channel === 'pr' || u?.current?.channel === 'local';

/** The status bar chip while running a PR build, or null. `title` is the PR title when known (the list was loaded). */
export function prChip(u, title = null) {
  const c = u?.current;
  if (c?.channel !== 'pr' || !c.pr) return null;
  return {
    text: `PR #${c.pr}`,
    title: `You run PR #${c.pr}'s build (${sha7(c.sha)})${title ? `: ${title}` : ''}. Click to change build or go back to main.`,
  };
}

/** The toast announcing a staged version, once per version (`lastToasted` is the persisted one). */
export function readyToast(u, lastToasted) {
  if (u?.state !== 'ready' || !u.available || u.available.version === lastToasted) return null;
  const v = u.available.version;
  return u.apply_on_quit
    ? `Orchestrator ${v} is ready. It installs when you quit.`
    : `Orchestrator ${v} is ready. It replaces your local build only if you restart now.`;
}

/** Palette entries' enabled state: `why` is the greyed-out hint. */
export function updateActions(u) {
  return {
    check: u?.enabled ? null : (u?.disabled_why ?? 'updates unavailable'),
    restart: u?.state === 'ready' ? null : 'no update ready',
  };
}

/** The outcome of a manual check, once the core settled (`checking`/`downloading` still run). */
export function checkOutcome(u) {
  if (!u) return null;
  switch (u.state) {
    case 'ready':
      return { text: `Orchestrator ${u.available.version} is ready` };
    case 'error':
      return { text: `Update check failed: ${u.error ?? 'unknown error'}`, error: true };
    case 'idle':
      return { text: `Orchestrator is up to date (${u.current.version})` };
    default:
      return null;
  }
}

/** "About Orchestrator" text: version, short SHA, channel, last check. */
export function aboutText(u) {
  const c = u?.current;
  if (!c) return 'Orchestrator';
  const sha = c.sha ? ` · ${c.sha.slice(0, 7)}` : '';
  const last = u.checked_at ? ` · checked ${clock(u.checked_at * 1000)}` : '';
  const channel = c.channel === 'main' ? 'main' : c.channel === 'pr' && c.pr ? `PR #${c.pr}` : 'local build';
  return `Orchestrator ${c.version}${sha} · ${channel}${last}`;
}

// ---------------------------------------------------------------- Settings › Builds (ux-brief)

export const sha7 = (s) => (s ?? '').slice(0, 7);

const sameSha = (a, b) => !!a && !!b && (a.startsWith(b) || b.startsWith(a));

/** "12 min ago", "just now". */
export function agoText(secs, now = Date.now()) {
  if (!secs) return '';
  const m = Math.floor((now - secs * 1000) / 60000);
  if (m < 1) return 'just now';
  if (m < 60) return `${m} min ago`;
  const h = Math.floor(m / 60);
  return h < 48 ? `${h} h ago` : `${Math.floor(h / 24)} d ago`;
}

/** The main update in words (header line 2 while running main). */
export function mainStateWords(u) {
  switch (u?.state) {
    case 'ready':
      return `${u.available?.version} ready${u.apply_on_quit ? ', installs when you quit' : ''}`;
    case 'downloading':
      return `downloading ${u.available?.version ?? 'the update'}${u.progress != null ? ` · ${pct(u.progress)} %` : ''}`;
    case 'checking':
      return 'checking for updates…';
    case 'error':
      return u.error ?? 'update failed';
    default:
      return 'up to date';
  }
}

/**
 * Back to main: `show` when running a non-main build; `why` is null when it can
 * run (the main update is `ready`), else the disabled hint. `canCheck` offers Check.
 */
export function backToMain(u) {
  if (!isNonMain(u)) return { show: false, why: 'you run main', canCheck: false };
  if (!u.enabled) return { show: true, why: u.disabled_why ?? 'updates unavailable', canCheck: false };
  switch (u.state) {
    case 'ready':
      return { show: true, why: null, canCheck: false };
    case 'downloading':
      return { show: true, why: 'main build downloading…', canCheck: false };
    case 'error':
      return { show: true, why: u.error ?? 'update failed', canCheck: true };
    default:
      return { show: true, why: 'checking main…', canCheck: true };
  }
}

/** The Settings › Builds header: the build you run. `title` = the running PR's title when known. */
export function buildsHeader(u, title = null) {
  const c = u?.current;
  if (!c) return null;
  const channel =
    c.channel === 'pr' && c.pr
      ? `PR #${c.pr}${title ? ` ${title}` : ''}`
      : c.channel === 'main'
        ? 'main'
        : 'local build';
  const back = backToMain(u);
  let line2;
  if (!back.show) line2 = mainStateWords(u);
  else if (u.state === 'ready') line2 = `main ${u.available?.version} is downloaded: Back to main restarts into it.`;
  else line2 = back.why;
  return { version: c.version, sha: sha7(c.sha), fullSha: c.sha ?? '', channel, kind: c.channel, line2, back };
}

/** The line above the list: "Open pull requests · refreshed 14:02". */
export const listLine = (list, refreshing) =>
  `Open pull requests · ${refreshing ? 'refreshing…' : list?.fetched_at ? `refreshed ${clock(list.fetched_at * 1000)}` : 'not loaded'}`;

/**
 * One PR row's state (ux-brief table). Returns
 * `{ kind, text, note?, tone?, action: { label, run: 'install'|'restart', primary?, disabled? } | null }`.
 * `kind`: installing | ready | error | running | running-newer | current | outdated | draft | none.
 */
export function rowState(pr, u, now = Date.now()) {
  const inst = u?.pr_install;
  const b = pr.build;
  const off = u && !u.enabled;
  if (inst?.pr === pr.number) {
    if (inst.state === 'downloading')
      return {
        kind: 'installing',
        text: `Downloading${inst.progress != null ? ` · ${pct(inst.progress)} %` : ''}`,
        action: null,
      };
    if (inst.state === 'ready')
      return { kind: 'ready', text: 'Ready', action: { label: 'Restart to run', run: 'restart', primary: true } };
    if (inst.state === 'error')
      return {
        kind: 'error',
        text: inst.error ?? 'install failed',
        tone: 'warn',
        action: { label: 'Retry', run: 'install', disabled: off },
      };
  }
  const c = u?.current;
  const busy = u?.pr_install?.state === 'downloading';
  const install = (label = 'Install') => ({ label, run: 'install', disabled: busy || off });
  if (c?.channel === 'pr' && c.pr === pr.number) {
    if (b && !sameSha(b.sha, c.sha))
      return { kind: 'running-newer', text: 'running · newer build', dot: true, action: install('Update') };
    return { kind: 'running', text: 'running', dot: true, action: null };
  }
  if (b)
    return b.status === 'current'
      ? {
          kind: 'current',
          text: ['build ' + sha7(b.sha), agoText(b.published_at, now)].filter(Boolean).join(' · '),
          action: install(),
        }
      : {
          kind: 'outdated',
          text: 'built from an older commit',
          note: '(newest commit not built yet)',
          action: install(),
        };
  return pr.draft
    ? { kind: 'draft', text: 'draft: not built', tone: 'muted', action: null }
    : { kind: 'none', text: 'no build yet', tone: 'muted', note: 'CI builds after the checks pass', action: null };
}

/** The inline confirm for "Restart to run" (ux-brief). */
export function restartConfirm(u, n) {
  const fromPr = u?.current?.channel === 'pr';
  const data = fromPr
    ? `Your data is not snapshotted again (the snapshot from before PR #${u.current.pr} is kept).`
    : 'Your data is snapshotted first; Back to main restores it if this PR changes the database.';
  return `Restart into PR #${n}? Agents are suspended and resume. ${data}`;
}

/** The "PR #n's build is ready" toast, once per staged build (`lastToasted` = `n:sha`). */
export function prReadyToast(u, lastToasted) {
  const i = u?.pr_install;
  if (i?.state !== 'ready' || prKey(i) === lastToasted) return null;
  return `PR #${i.pr}'s build is ready.`;
}
export const prKey = (i) => `${i.pr}:${i.sha}`;

/** Start-up warnings that announce a restore (api-contract §6): amber toast, not the banner. */
export const isRestoreWarning = (w) => typeof w === 'string' && w.startsWith('Your data was restored from before PR #');
