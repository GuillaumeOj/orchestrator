// Feature docs location (change-how-specs-are-saved §3.1, §3.6): the Project settings section,
// the helpers behind it, and the location line on the Feature page, against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { fixture } from './fixture.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return r instanceof Error ? Promise.reject(r) : Promise.resolve(typeof r === 'function' ? r(args) : (r ?? null));
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));

const { default: ProjectSettingsWindow } = await import('../src/components/ProjectSettingsWindow.svelte');
const { default: FeaturePane } = await import('../src/panes/FeaturePane.svelte');
const fd = await import('../src/lib/featuredocs.js');
const { docRepoPath } = await import('../src/lib/teams.js');
const { featureDoc, refKind } = await import('../src/lib/overwatch.js');

const called = (cmd) => calls.filter(([c]) => c === cmd);
const el = (x) => /** @type {any} */ (x);
const txt = (c) => c.textContent.replace(/\s+/g, ' ');

const stored = (feature_docs) => ({
  settings: feature_docs ? { feature_docs, notion_parent_page: 'abc' } : { notion_parent_page: 'abc' },
  backend: { type: 'local' },
  track_orchestrator_dir: true,
});

async function open(feature_docs) {
  replies.project_settings = stored(feature_docs);
  app.data = fixture();
  app.window = { kind: 'project-settings', projectId: 'shop-api', section: 'feature-docs' };
  const r = render(ProjectSettingsWindow);
  await waitFor(() => expect(r.container.querySelector('[data-row="docs"]')).toBeTruthy());
  return r;
}
const docsInput = (c) => el(c.querySelector('input[aria-label="Docs folder"]'));
const designsInput = (c) => el(c.querySelector('input[aria-label="Designs folder"]'));
const radio = (c, row, label) =>
  el([...c.querySelectorAll(`[data-row="${row}"] label`)].find((l) => txt(l).trim() === label)?.querySelector('input'));
const saveBtn = (c) => el([...c.querySelectorAll('.sfoot button')].find((b) => b.textContent === 'Save'));

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  replies.project_settings_set = (a) => ({ ...a.form, settings: a.form.settings });
  replies.feature_docs_reveal = null;
});

describe('feature docs helpers', () => {
  it('checkPath mirrors the backend rules and messages', () => {
    expect(fd.checkPath('docs/specs/{slug}')).toBeNull();
    expect(fd.checkPath('docs/{slug}/')).toBeNull();
    expect(fd.checkPath('')).toBe('Must contain {slug}');
    expect(fd.checkPath('docs/specs')).toBe('Must contain {slug}');
    for (const bad of [
      '/abs/{slug}',
      '../x/{slug}',
      'a/.git/{slug}',
      'a/.GIT/{slug}',
      './{slug}',
      'a//{slug}',
      'a\\{slug}',
      '~/{slug}',
      'a/{other}/{slug}',
    ]) {
      expect(fd.checkPath(bad), bad).toBe('Must be a path inside the repository');
    }
    expect(fd.checkPath(`${'a'.repeat(200)}/{slug}`)).toBe('Too long (200 characters at most)');
  });

  it('resolveSetting fills the slug; with-docs follows the docs folder', () => {
    expect(fd.resolveSetting(null, 'checkout')).toEqual({
      docs: { kind: 'repo', dir: '.orchestrator/features/checkout' },
      designs: { kind: 'repo', dir: '.orchestrator/features/checkout/design' },
    });
    const r = fd.resolveSetting({ docs: { kind: 'app' }, designs: { kind: 'with_docs' } }, 'checkout');
    expect(r.docs.kind).toBe('app');
    expect(r.designs).toEqual({ kind: 'app', dir: null });
  });

  it('locationOf: the snapshot, else legacy; designsWithDocs', () => {
    const l = fd.locationOf({ slug: 'co' }, null);
    expect(l.docs.dir).toBe('.orchestrator/features/co');
    expect(fd.designsWithDocs(l)).toBe(true);
    const o = fd.locationOf(
      { slug: 'co' },
      { docs: { kind: 'repo', dir: 'docs/x' }, designs: { kind: 'repo', dir: 'docs/d' } },
    );
    expect(fd.designsWithDocs(o)).toBe(false);
  });

  it('docRepoPath follows the docs folder; null when not saved in the repo', () => {
    const f = { slug: 'checkout', integration_worktree: '/w' };
    const loc = {
      docs: { kind: 'repo', dir: 'docs/specs/checkout' },
      designs: { kind: 'repo', dir: 'docs/specs/checkout/design' },
    };
    expect(docRepoPath(f, 'spec.md', loc)).toBe('docs/specs/checkout/spec.md');
    expect(docRepoPath(f, 'spec.md')).toBe('.orchestrator/features/checkout/spec.md');
    expect(
      docRepoPath(f, 'spec.md', {
        docs: { kind: 'app', dir: '/app/docs' },
        designs: { kind: 'app', dir: '/app/docs/design' },
      }),
    ).toBeNull();
  });

  it('featureDoc finds the feature by its docs folder, else the legacy path', () => {
    const features = [{ slug: 'checkout', docs_location: { docs: { kind: 'repo', dir: 'docs/specs/checkout' } } }];
    expect(featureDoc('docs/specs/checkout/spec.md', features)).toEqual({ slug: 'checkout', name: 'spec.md' });
    // A path prefix, not a substring: a `{slug}`-first template cannot claim a code path.
    expect(featureDoc('/w/co/docs/specs/checkout/spec.md', features)).toBeNull();
    const first = [{ slug: 'checkout', docs_location: { docs: { kind: 'repo', dir: 'checkout/docs' } } }];
    expect(featureDoc('checkout/docs/spec.md', first)).toEqual({ slug: 'checkout', name: 'spec.md' });
    expect(featureDoc('src/checkout/docs/spec.md', first)).toBeNull();
    expect(featureDoc('.orchestrator/features/x/spec.md', features)).toEqual({ slug: 'x', name: 'spec.md' });
    expect(featureDoc('docs/other.md', features)).toBeNull();
  });
});

describe('Overwatch links to docs kept outside the repository', () => {
  const app = '/Users/me/Library/Application Support/Orchestrator/projects/p1/features/checkout';
  const features = [
    {
      slug: 'checkout',
      docs_location: {
        docs: { kind: 'app', dir: `${app}/docs` },
        designs: { kind: 'app', dir: `${app}/design` },
      },
    },
  ];
  const doc = `${app}/docs/spec.md`;

  it('accepts a spaced absolute path in backticks or angle brackets, or percent-encoded', () => {
    const want = { kind: 'doc', path: doc, abs: true, name: 'spec.md', section: null };
    expect(refKind(`\`${doc}\``)).toEqual(want);
    expect(refKind(`<${doc}>`)).toEqual(want);
    expect(refKind(doc.replaceAll(' ', '%20'))).toEqual(want);
    expect(refKind(`\`${doc}#scope\``)).toEqual({ ...want, section: 'scope' });
    expect(featureDoc(doc, features, true)).toEqual({ slug: 'checkout', name: 'spec.md' });
    expect(featureDoc(`${app}/design/home.md`, features, true)).toEqual({
      slug: 'checkout',
      name: 'home.md',
      design: true,
    });
  });

  it('refuses a bare absolute path and traversal', () => {
    expect(refKind('/etc/notes.md')).toBeNull();
    expect(refKind('`/a/../etc/notes.md`')).toBeNull();
    expect(refKind('`/etc/passwd`')).toBeNull();
  });

  it('does not link an absolute path outside any feature folder', () => {
    const r = /** @type {{ abs: boolean, path: string }} */ (
      refKind('`/Users/me/Library/Application Support/Orchestrator/other/spec.md`')
    );
    expect(r.abs).toBe(true);
    expect(featureDoc(r.path, features, true)).toBeNull();
    expect(featureDoc('/x/.orchestrator/features/a/spec.md', features, true)).toBeNull();
  });
});

describe('Project settings › Feature docs', () => {
  it('shows the defaults with the resolved examples and a disabled Save', async () => {
    const { container } = await open(null);
    expect(docsInput(container).value).toBe('.orchestrator/features/{slug}');
    expect(txt(container)).toContain('→ .orchestrator/features/checkout/');
    expect(txt(container)).toContain('→ .orchestrator/features/checkout/design/');
    expect(txt(container)).toContain("{slug} is the feature's short name");
    expect(txt(container)).toContain('Applies to features you start from now on. Running features keep their folders.');
    expect(radio(container, 'docs', 'In the repository').checked).toBe(true);
    expect(radio(container, 'designs', 'With the docs').checked).toBe(true);
    expect(designsInput(container)).toBeNull();
    expect(txt(container)).not.toContain('Unsaved changes');
    expect(saveBtn(container).disabled).toBe(true);
  });

  it('is in the nav after Files', async () => {
    const { container } = await open(null);
    const tabs = [...container.querySelectorAll('[role="tab"]')].map((t) => t.textContent.trim());
    expect(tabs.indexOf('Feature docs')).toBe(tabs.indexOf('Files') + 1);
  });

  it('switching options shows the right fields and helpers', async () => {
    const { container } = await open(null);
    await fireEvent.click(radio(container, 'designs', 'In the repository'));
    expect(designsInput(container)).toBeTruthy();
    await fireEvent.input(designsInput(container), { target: { value: 'docs/design/{slug}' } });
    expect(txt(container)).toContain('→ docs/design/checkout/');
    await fireEvent.click(radio(container, 'docs', 'Not saved in the repository'));
    expect(docsInput(container)).toBeNull();
    expect(txt(container)).toContain('Kept on this Mac only, never committed. Deleted with the feature.');
    await fireEvent.click(radio(container, 'designs', 'With the docs'));
    expect(txt(container)).toContain('→ Not saved either');
  });

  it('prefills a suggested designs folder when picking In the repository', async () => {
    const { container } = await open(null);
    await fireEvent.click(radio(container, 'designs', 'In the repository'));
    expect(designsInput(container).value).toBe('.orchestrator/features/{slug}/design');
    expect(saveBtn(container).disabled).toBe(false);
  });

  it('shows validation errors under the field, disables Save and says why', async () => {
    const { container } = await open(null);
    await fireEvent.input(docsInput(container), { target: { value: 'docs/specs' } });
    expect(txt(container)).toContain('Must contain {slug}');
    expect(docsInput(container).classList.contains('bad')).toBe(true);
    expect(txt(container)).toContain('Fix the folders above to save.');
    expect(txt(container)).toContain('Unsaved changes');
    expect(saveBtn(container).disabled).toBe(true);
    await fireEvent.click(radio(container, 'designs', 'In the repository'));
    await fireEvent.input(designsInput(container), { target: { value: '../design/{slug}' } });
    expect(txt(container)).toContain('Must be a path inside the repository');
    await fireEvent.input(docsInput(container), { target: { value: 'docs/specs/{slug}' } });
    await fireEvent.input(designsInput(container), { target: { value: 'docs/design/{slug}' } });
    expect(txt(container)).not.toContain('Fix the folders above');
    expect(saveBtn(container).disabled).toBe(false);
  });

  it('saves the setting, keeping the other settings as stored now', async () => {
    const { container } = await open(null);
    await fireEvent.input(docsInput(container), { target: { value: 'docs/specs/{slug}/' } });
    await fireEvent.click(radio(container, 'designs', 'Not saved in the repository'));
    await fireEvent.click(saveBtn(container));
    await waitFor(() => expect(called('project_settings_set')).toHaveLength(1));
    expect(called('project_settings_set')[0][1]).toEqual({
      project: 'shop-api',
      form: {
        settings: {
          notion_parent_page: 'abc',
          feature_docs: { docs: { kind: 'repo', path: 'docs/specs/{slug}' }, designs: { kind: 'app' } },
        },
        backend: { type: 'local' },
        track_orchestrator_dir: true,
      },
    });
    await waitFor(() => expect(txt(container)).not.toContain('Unsaved changes'));
  });

  it('⌘⏎ saves; Revert goes back to what is saved', async () => {
    const { container } = await open({ docs: { kind: 'repo', path: 'docs/{slug}' }, designs: { kind: 'with_docs' } });
    expect(docsInput(container).value).toBe('docs/{slug}');
    await fireEvent.input(docsInput(container), { target: { value: 'x/{slug}' } });
    await fireEvent.click(el([...container.querySelectorAll('.sfoot button')].find((b) => b.textContent === 'Revert')));
    expect(docsInput(container).value).toBe('docs/{slug}');
    await fireEvent.input(docsInput(container), { target: { value: 'x/{slug}' } });
    await fireEvent.keyDown(container.querySelector('.ps'), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(called('project_settings_set')).toHaveLength(1));
  });

  it('puts a backend error under the field it names', async () => {
    replies.project_settings_set = new Error('feature_docs.designs.path: Must be a path inside the repository');
    const { container } = await open(null);
    await fireEvent.click(radio(container, 'designs', 'In the repository'));
    await fireEvent.input(designsInput(container), { target: { value: 'docs/design/{slug}' } });
    await fireEvent.click(saveBtn(container));
    await waitFor(() =>
      expect(txt(container.querySelector('[data-row="designs"]'))).toContain('Must be a path inside the repository'),
    );
    expect(txt(container.querySelector('[data-row="docs"]'))).not.toContain('Must be a path');
    // Typing again clears it.
    await fireEvent.input(designsInput(container), { target: { value: 'docs/design/{slug}/x' } });
    expect(txt(container)).not.toContain('Must be a path inside the repository');
  });
});

describe('Feature page · where the docs live', () => {
  const view = (docs_location, extra = {}) => ({
    feature: { id: 'f1', project_id: 'shop-api', slug: 'checkout', integration_worktree: '/w/co' },
    run: null,
    team: null,
    project: { id: 'shop-api', name: 'shop-api', backend: 'local' },
    agents: [],
    tasks: [],
    docs: ['spec.md'],
    artifacts: [],
    timeline: [],
    docs_location,
    ...extra,
  });
  const show = async (v) => {
    replies.feature_view = v;
    replies.feature_doc = '# Doc';
    app.data = fixture();
    app.panes = { f1: { type: 'feature', featureId: 'f1' } };
    const r = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => expect(r.container.querySelector('[data-docs-location]')).toBeTruthy());
    return r;
  };
  const line = (c) => txt(c.querySelector('[data-docs-location]'));

  it('docs in the repository: the folder', async () => {
    const { container } = await show(
      view({
        docs: { kind: 'repo', dir: 'docs/specs/checkout' },
        designs: { kind: 'repo', dir: 'docs/specs/checkout/design' },
      }),
    );
    expect(line(container)).toContain('In the repository · docs/specs/checkout/');
    expect(line(container)).not.toContain('Designs');
    expect(container.querySelector('[data-docs-location] button')).toBeNull();
  });

  it('a legacy feature (no location) shows the default folder', async () => {
    const { container } = await show(view(undefined));
    expect(line(container)).toContain('In the repository · .orchestrator/features/checkout/');
  });

  it('not saved: says so, Reveal in Finder opens the app folder', async () => {
    const { container } = await show(
      view({ docs: { kind: 'app', dir: '/app/docs' }, designs: { kind: 'app', dir: '/app/docs/design' } }),
    );
    expect(line(container)).toContain('Not saved in the repository');
    const b = el(
      [...container.querySelectorAll('[data-docs-location] button')].find((x) => x.textContent === 'Reveal in Finder'),
    );
    await fireEvent.click(b);
    expect(called('feature_docs_reveal')[0][1]).toEqual({ featureId: 'f1', which: 'docs' });
    // No repo path line for a doc that is not in the repo.
    expect(container.querySelector('[data-docpath]')).toBeNull();
  });

  it('designs With the docs while docs are not saved: the docs line only', async () => {
    const { container } = await show(
      view({ docs: { kind: 'app', dir: '/app/docs' }, designs: { kind: 'app', dir: '/app/docs/design' } }),
    );
    expect(container.querySelector('[data-loc="designs"]')).toBeNull();
  });

  it('designs outside the docs folder get their own line', async () => {
    const { container } = await show(
      view({ docs: { kind: 'repo', dir: 'docs/specs/checkout' }, designs: { kind: 'app', dir: '/app/design' } }),
    );
    expect(line(container)).toContain('Designs · Not saved in the repository');
    const b = container.querySelector('[data-loc="designs"] button');
    await fireEvent.click(b);
    expect(called('feature_docs_reveal')[0][1]).toEqual({ featureId: 'f1', which: 'designs' });
    cleanup();
    const r2 = await show(
      view({
        docs: { kind: 'repo', dir: 'docs/specs/checkout' },
        designs: { kind: 'repo', dir: 'docs/design/checkout' },
      }),
    );
    expect(line(r2.container)).toContain('Designs · In the repository · docs/design/checkout/');
  });
});
