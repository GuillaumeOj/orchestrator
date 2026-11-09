// A dock shell's live title (design-notes §6): `<shell> · <cwd>`. Where the
// directory comes from, in order: OSC 7 (instant, when the shell sends it),
// the core's `term_cwd` poll (every shell), an OSC 0/2 title that looks like a
// path, the path the shell opened in. Pure helpers here; the wiring is in
// `app.svelte.js` (`trackShells`, `readShellCwd`, `onShellOsc7`, `onShellTitle`).

import { shortPath } from './format.js';

/** Between two `term_cwd` reads of a visible shell (ms). */
export const POLL_MS = 1500;
/** A cwd longer than this is cut from the left in the tab. */
export const TITLE_MAX = 32;

/**
 * An OSC 7 payload (`file://host/%2Fpath`) → `{ host, path }`, or null when it
 * is not a file URL. The path is percent-decoded.
 */
export function parseOsc7(data) {
  let url;
  try {
    url = new URL(String(data));
  } catch {
    return null;
  }
  if (url.protocol !== 'file:') return null;
  let path;
  try {
    path = decodeURIComponent(url.pathname);
  } catch {
    return null;
  }
  return path.startsWith('/') ? { host: url.hostname.toLowerCase(), path } : null;
}

/**
 * Whether an OSC 7 host is this machine: none, `localhost`, or one of `local`
 * (the names this terminal's shell has reported before: its first OSC 7 comes
 * from the local prompt, before any `ssh`). An `ssh` session's OSC 7 names the
 * remote host, and its path means nothing here.
 */
export const isLocalHost = (host, local = []) => !host || host === 'localhost' || local.includes(host);

/** Whether an OSC 0/2 title is a directory (`~/dev`, `/tmp`), not `user@host:path`, `zsh` or a command. */
export const looksLikePath = (title) => /^[~/]/.test(String(title ?? '').trim());

/** `…prove-orchestrator-ui-ux/ui-1`: the end of `path`, `max` characters at most. */
export function truncateLeft(path, max = TITLE_MAX) {
  const p = String(path ?? '');
  return p.length > max ? `…${p.slice(p.length - (max - 1))}` : p;
}

/** The directory as shown: home as `~`, cut from the left. */
export const shownCwd = (cwd, home) => truncateLeft(shortPath(cwd, home) || '~');

/** `zsh · ~/dev/orch/ui/src/components`. */
export const liveTitle = (shell, cwd, home) => `${shell || 'zsh'} · ${shownCwd(cwd, home)}`;

/** The tooltip's two lines: the title, then the full path. */
export const shellTip = (title, cwd) => (cwd ? { label: title, detail: cwd } : (title ?? ''));
