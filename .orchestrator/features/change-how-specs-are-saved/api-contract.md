# API contract — Feature docs location

## 1. Project setting (`ProjectSettings.feature_docs`)

Stored in the project's `settings` JSON (no migration for it). Absent = defaults.

```json
"feature_docs": {
  "docs":    { "kind": "repo", "path": ".orchestrator/features/{slug}" },
  "designs": { "kind": "with_docs" }
}
```

- `docs.kind`: `"repo"` (with `path`) | `"app"` (not saved in the repository).
- `designs.kind`: `"with_docs"` | `"repo"` (with `path`) | `"app"`.
- `path`: relative to the repo root, contains `{slug}`, no `..`, no absolute, no `.git`
  component, no empty component, ≤ 200 chars. A trailing `/` is dropped.

Read: `project_settings_get` returns it inside `settings` (filled with the defaults when
absent, so the UI always gets both rows).

Write: `project_settings_set { project, form: { settings, backend, track_orchestrator_dir } }`
as today, `settings.feature_docs` included. An invalid value fails the whole save with an
error string `feature_docs.<docs|designs>.path: <message>`, where `<message>` is exactly
the UI copy: `Must contain {slug}` (also for an empty path) · `Must be a path inside the
repository` (absolute, `..`, `.git` in any case, `.`, backslash, empty part, a token other
than `{slug}`) · `Too long (200 characters at most)`. The UI shows `<message>` under that
field; it also runs the same checks itself before saving. Nothing is saved.

## 2. Feature snapshot

At feature creation the backend resolves and stores the locations on the feature
(new nullable column `features.docs_location`, JSON; `NULL` = legacy defaults):

```json
{ "docs":    { "kind": "repo", "dir": "docs/specs/checkout" },
  "designs": { "kind": "app",  "dir": "/Users/…/Orchestrator/projects/<id>/features/checkout/design" } }
```

`repo` dirs are relative to a worktree; `app` dirs are absolute. With `<app>` =
`<app data>/projects/<id>/features/<slug>`: docs `app` → `<app>/docs`; designs
`with_docs` → `<docs dir>/design` in every case (so `<app>/docs/design` when docs are not
saved); designs `app` with docs in the repo → `<app>/design`. One resolver in
`agent-runtime` (`FeatureDocs::of(feature) -> { docs: Loc, designs: Loc }` with
`dir_in(worktree)`) replaces every hard-coded `.orchestrator/features/<slug>` read.

## 3. Feature pane (`run_json`)

Adds `docs_location` next to `docs`:

```json
"docs_location": {
  "docs":    { "kind": "repo", "dir": "docs/specs/checkout" },
  "designs": { "kind": "repo", "dir": "docs/specs/checkout/design" }
}
```

`docs` keeps its shape (file names); `read_feature_doc` / `read_feature_asset` keep their
arguments and resolve through the snapshot.

## 4. Reveal command

`feature_docs_reveal { feature_id, which: "docs" | "designs" }` → opens the app folder in
Finder (creates it if missing). Refused for a `repo` location.

## 5. Agents

- Lead brief / protocol / `handoff` description / designers' instructions use the
  resolved dirs (repo: relative path; app: absolute path, "do not commit").
- `app` dirs are added to each run agent's sandbox `allowWrite` and to its prompt's file
  list at spawn; the hard policy allows writes there.
- `handoff` with kind `design` accepts an absolute path only when its canonical path is
  inside the feature's `app` docs or designs dir.
- `request_spec_approval`: docs listed from the resolved docs dir; the clean-worktree
  check stays; `doc_lines[].path` is the repo-relative or absolute path.
