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
