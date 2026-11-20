// Where a feature's docs and designs are saved (change-how-specs-are-saved, api-contract §1–§3):
// the project's setting (`settings.feature_docs`), its checks (the backend's `check_path`, same
// messages), the resolved example, and the folders a feature or a launch resolves to.

export const DEFAULT_DOCS_PATH = '.orchestrator/features/{slug}';
export const MAX_PATH_LEN = 200;
/** What the project has until it chooses (`settings.feature_docs` absent). */
export const DEFAULT_FEATURE_DOCS = Object.freeze({
  docs: { kind: 'repo', path: DEFAULT_DOCS_PATH },
  designs: { kind: 'with_docs' },
});

const INSIDE = 'Must be a path inside the repository';
const NEEDS_SLUG = 'Must contain {slug}';

/** The error under a folder template, null when it is valid. Mirrors `check_path` (orch-team). */
export function checkPath(raw) {
  const p = String(raw ?? '')
    .trim()
    .replace(/\/+$/, '');
  if (!p) return NEEDS_SLUG;
  if ([...p].length > MAX_PATH_LEN) return `Too long (${MAX_PATH_LEN} characters at most)`;
  if (p.startsWith('/') || p.startsWith('~') || /[\\\n\r\0]/.test(p)) return INSIDE;
  if (p.split('/').some((c) => c === '' || c === '..' || c === '.' || c.toLowerCase() === '.git')) return INSIDE;
  if (/[{}]/.test(p.replaceAll('{slug}', ''))) return INSIDE;
  if (!p.includes('{slug}')) return NEEDS_SLUG;
  return null;
}

/** A setting as the form edits it: both rows with a path each, so switching an option keeps the typed one. */
export function toDraft(fd) {
  const s = fd ?? DEFAULT_FEATURE_DOCS;
  return {
    docs: { kind: s.docs?.kind ?? 'repo', path: s.docs?.path ?? DEFAULT_DOCS_PATH },
    designs: { kind: s.designs?.kind ?? 'with_docs', path: s.designs?.path ?? '' },
  };
}

/** The draft as `settings.feature_docs` (a path only under "In the repository", trailing "/" dropped). */
export function fromDraft(d) {
  const clean = (p) => p.trim().replace(/\/+$/, '');
  return {
    docs: d.docs.kind === 'repo' ? { kind: 'repo', path: clean(d.docs.path) } : { kind: 'app' },
    designs: d.designs.kind === 'repo' ? { kind: 'repo', path: clean(d.designs.path) } : { kind: d.designs.kind },
  };
}

/** The errors of a draft, per row: `{ docs?, designs? }` (only rows with a path field can fail). */
export function draftErrors(d) {
  const e = {};
  if (d.docs.kind === 'repo') e.docs = checkPath(d.docs.path);
  if (d.designs.kind === 'repo') e.designs = checkPath(d.designs.path);
  return Object.fromEntries(Object.entries(e).filter(([, v]) => v));
}

/** Two drafts that save the same value. */
export function sameDraft(a, b) {
  return JSON.stringify(fromDraft(a)) === JSON.stringify(fromDraft(b));
}

/** Maps a backend error `feature_docs.<docs|designs>.path: <message>` to `{ row, message }`, else null. */
export function parseFieldError(text) {
  const m = /feature_docs\.(docs|designs)\.path:\s*(.+)$/s.exec(String(text ?? ''));
  return m ? { row: m[1], message: m[2].trim() } : null;
}

const fill = (p, slug) => p.trim().replace(/\/+$/, '').replaceAll('{slug}', slug);

/** The folders a setting gives a feature named `slug`, as the section shows them:
 *  `{ docs: { kind, dir }, designs: { kind, dir } }`; an `app` dir is null (the app picks it). */
export function resolveSetting(fd, slug) {
  const s = fd ?? DEFAULT_FEATURE_DOCS;
  const docs =
    s.docs?.kind === 'app'
      ? { kind: 'app', dir: null }
      : { kind: 'repo', dir: fill(s.docs?.path ?? DEFAULT_DOCS_PATH, slug) };
  let designs;
  if (s.designs?.kind === 'repo') designs = { kind: 'repo', dir: fill(s.designs.path ?? '', slug) };
  else if (s.designs?.kind === 'app') designs = { kind: 'app', dir: null };
  else designs = { kind: docs.kind, dir: docs.dir === null ? null : `${docs.dir}/design` };
  return { docs, designs };
}

/** A feature's folders (run_json `docs_location`, else the feature's own snapshot), the legacy ones
 *  for a feature created before the setting. */
export function locationOf(feature, loc) {
  const l = loc ?? feature?.docs_location;
  if (l?.docs?.dir) return l;
  const docs = `.orchestrator/features/${feature?.slug}`;
  return { docs: { kind: 'repo', dir: docs }, designs: { kind: 'repo', dir: `${docs}/design` } };
}

/** The designs folder is the docs folder's `design/`: the Feature page names the docs line only. */
export function designsWithDocs(l) {
  return l.designs.kind === l.docs.kind && l.designs.dir === `${l.docs.dir}/design`;
}

/** "name/" with one trailing slash. */
export const slashed = (dir) => `${String(dir).replace(/\/+$/, '')}/`;
