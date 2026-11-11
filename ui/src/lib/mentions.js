// @mentions of features in the Overwatch input and thread (add-mention-in-overwatch
// spec §3.2, §3.3, §8, §9). Pure: tested in test/mentions.test.js.
//
// A token is `@` + chars of `[A-Za-z0-9._/-]`, ending at the first other char, then
// trailing `. / - _` trimmed. It starts at the start of the text or after whitespace or
// `(` `[` `"` `'`, and never inside an inline code span (odd number of backticks before
// it on its line).

import { score } from './fuzzy.js';
import { runFinished } from './teams.js';

export const MAX_MENTIONS = 10;

const TOKEN_CHAR = /[A-Za-z0-9._/-]/;
const BEFORE_AT = /[\s(["']/;
const TRAIL = /[./\-_]+$/;

const isCurrent = (f) => !!f.run && !runFinished(f.run.status);
const live = (features) => (features ?? []).filter((f) => !f.archived);

/** Is the `@` at `i` a mention start: after a boundary and not in an inline code span. */
function startsToken(text, i) {
  if (text[i] !== '@') return false;
  if (i > 0 && !BEFORE_AT.test(text[i - 1])) return false;
  const line = text.slice(text.lastIndexOf('\n', i - 1) + 1, i);
  return (line.match(/`/g)?.length ?? 0) % 2 === 0;
}

/** The token being typed: `{ start, query }` (`start` = the `@`), or null when the caret is not in one. */
export function tokenAtCaret(text, caret) {
  let i = caret;
  while (i > 0 && TOKEN_CHAR.test(text[i - 1])) i--;
  if (i === 0 || text[i - 1] !== '@') return null;
  const start = i - 1;
  return startsToken(text, start) ? { start, query: text.slice(i, caret) } : null;
}

/** Every complete token of a text: `{ start, end, token }`, `token` with its `@`, trimmed (spec §8). */
export function tokensIn(text) {
  const out = [];
  for (let i = text.indexOf('@'); i >= 0; i = text.indexOf('@', i + 1)) {
    if (!startsToken(text, i)) continue;
    let j = i + 1;
    while (j < text.length && TOKEN_CHAR.test(text[j])) j++;
    const body = text.slice(i + 1, j).replace(TRAIL, '');
    if (body) out.push({ start: i, end: i + 1 + body.length, token: `@${body}` });
  }
  return out;
}

/** A project's key: its name lowercased, runs outside `[a-z0-9._-]` as `-`, trimmed of `-` (spec §8). */
export const projectKey = (name) =>
  String(name ?? '')
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '');

/**
 * The tokens of the non-archived features: `Map<feature id, { token, key }>`. `@slug`, or
 * `@<project key>/<slug>` when the slug is shared; the key falls back to the project id
 * when empty or shared by two projects with non-archived features (spec §8, §9).
 */
export function tokenTable(features, projects) {
  const fs = live(features);
  const name = new Map((projects ?? []).map((p) => [p.id, p.name]));
  const keyOwners = new Map();
  for (const f of fs) {
    const k = projectKey(name.get(f.project_id) ?? f.project_id);
    if (!keyOwners.has(k)) keyOwners.set(k, new Set());
    keyOwners.get(k).add(f.project_id);
  }
  const slugs = new Map();
  for (const f of fs) slugs.set(f.slug, (slugs.get(f.slug) ?? 0) + 1);
  const table = new Map();
  for (const f of fs) {
    const k = projectKey(name.get(f.project_id) ?? f.project_id);
    const key = !k || keyOwners.get(k).size > 1 ? f.project_id : k;
    table.set(f.id, { key, token: slugs.get(f.slug) > 1 ? `@${key}/${f.slug}` : `@${f.slug}` });
  }
  return table;
}

/** The positions of `query` in `text`, as `score` walks them (in order, lowercase), or null. */
export function matchPositions(query, text) {
  const q = query.toLowerCase().replace(/\s+/g, '');
  const t = text.toLowerCase();
  const out = [];
  let ti = 0;
  for (const c of q) {
    const at = t.indexOf(c, ti);
    if (at < 0) return null;
    out.push(at);
    ti = at + 1;
  }
  return out;
}

/** `text` cut in `{ t, on }` runs, `on` at the matched positions (the row's emphasis). */
export function emphasis(text, positions) {
  if (!positions?.length) return [{ t: text, on: false }];
  const at = new Set(positions);
  const out = [];
  for (let i = 0; i < text.length; i++) {
    const on = at.has(i);
    if (out.length && out.at(-1).on === on) out.at(-1).t += text[i];
    else out.push({ t: text[i], on });
  }
  return out;
}

/**
 * The popup's rows for `query`: `{ current, other, rows }`, each row `{ id, f, token, key, project,
 * current, slug, title }` (`slug` / `title`: the emphasis runs). Current first; within a group by
 * score of `slug`, `title`, `<key>/<slug>` then project name; no query: by creation, newest first.
 */
export function candidates(features, projects, query = '') {
  const name = new Map((projects ?? []).map((p) => [p.id, p.name]));
  const table = tokenTable(features, projects);
  const q = query.trim();
  const rows = [];
  live(features).forEach((f, order) => {
    const { token, key } = table.get(f.id);
    const s = q ? Math.max(score(q, f.slug), score(q, f.title ?? ''), score(q, `${key}/${f.slug}`)) : 1;
    if (s <= 0) return;
    const bySlug = q ? matchPositions(q, f.slug) : null;
    const byTitle = q && !bySlug ? matchPositions(q, f.title ?? '') : null;
    rows.push({
      id: f.id,
      f,
      token,
      key,
      s,
      order,
      project: name.get(f.project_id) ?? f.project_id,
      current: isCurrent(f),
      slug: emphasis(f.slug, bySlug),
      title: emphasis(f.title ?? '', byTitle),
    });
  });
  const cmp = q
    ? (a, b) => b.s - a.s || a.project.localeCompare(b.project) || a.order - b.order
    : (a, b) => (b.f.created_at_ms ?? 0) - (a.f.created_at_ms ?? 0) || a.order - b.order;
  rows.sort(cmp);
  const current = rows.filter((r) => r.current);
  const other = rows.filter((r) => !r.current);
  return { current, other, rows: [...current, ...other] };
}

/** `text` with the `@query` at `start..caret` replaced by `token` + one space (none added if one follows).
 *  A caret inside a token also takes the rest of it, so no stray tail is left after the pick. */
export function insertToken(text, start, caret, token) {
  let end = caret;
  while (end < text.length && TOKEN_CHAR.test(text[end])) end++;
  const after = text.slice(end);
  const space = after.startsWith(' ') ? '' : ' ';
  const head = `${text.slice(0, start)}${token}${space}`;
  return { text: head + after, caret: head.length + (space ? 0 : 1) };
}

/** The snapshot a mention carries (api-contract §1 entry meta). */
export const snapshotOf = (f, projects) => ({
  kind: 'feature',
  id: f.id,
  slug: f.slug,
  title: f.title ?? '',
  project_id: f.project_id,
  project: (projects ?? []).find((p) => p.id === f.project_id)?.name ?? f.project_id,
});

/**
 * The mentions a message sends: the remembered picks whose token is still in the text, plus
 * hand-typed tokens that name exactly one non-archived feature (`@slug`, `@<key>/<slug>`,
 * `@<project id>/<slug>`). Deduped, at most 10; snapshots, in text order.
 */
export function mentionsToSend(text, picks, features, projects) {
  const fs = live(features);
  const byId = new Map(fs.map((f) => [f.id, f]));
  const table = tokenTable(features, projects);
  const forms = new Map(); // token -> feature ids
  const add = (token, id) => forms.set(token, [...(forms.get(token) ?? []), id]);
  for (const f of fs) {
    const { key } = table.get(f.id);
    add(`@${f.slug}`, f.id);
    add(`@${key}/${f.slug}`, f.id);
    add(`@${f.project_id}/${f.slug}`, f.id);
  }
  const picked = new Map((picks ?? []).map((p) => [p.token, p.id]));
  const ids = [];
  for (const { token } of tokensIn(text)) {
    const id = picked.get(token) ?? (new Set(forms.get(token)).size === 1 ? forms.get(token)[0] : null);
    if (id && byId.has(id) && !ids.includes(id)) ids.push(id);
  }
  return ids.slice(0, MAX_MENTIONS).map((id) => snapshotOf(byId.get(id), projects));
}

/** The forms a mention's token can take in a sent text: `@slug`, `@<key>/<slug>`, `@<project id>/<slug>`. */
export const mentionForms = (m) => [`@${m.slug}`, `@${projectKey(m.project)}/${m.slug}`, `@${m.project_id}/${m.slug}`];

/**
 * A user text cut in runs for the thread: `{ t }` plain, `{ t, m }` a mention (`m` = the
 * `meta.mentions` item). Whole trimmed tokens only, so `@add` never matches inside `@add-mention`.
 */
export function mentionRuns(text, mentions) {
  const byToken = new Map();
  for (const m of mentions ?? []) for (const t of mentionForms(m)) if (!byToken.has(t)) byToken.set(t, m);
  const runs = [];
  let at = 0;
  for (const { start, end, token } of tokensIn(text)) {
    const m = byToken.get(token);
    if (!m) continue;
    if (start > at) runs.push({ t: text.slice(at, start) });
    runs.push({ t: token, m });
    at = end;
  }
  if (at < text.length) runs.push({ t: text.slice(at) });
  return runs;
}

/** The announcement for the live region: `3 features`, `No feature matches`, `No features yet`. */
export function countText(n, anyFeature) {
  if (!anyFeature) return 'No features yet';
  return n ? `${n} feature${n === 1 ? '' : 's'}` : 'No feature matches';
}
