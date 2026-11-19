# Change how specs are saved

## 1. Problem

Every team run writes its feature docs (`spec.md`, `ux-brief.md`, `api-contract.md`,
`decisions.md`, …) and its design exports (`design/*.png|svg`) to
`.orchestrator/features/<slug>/`, commits them on the integration branch, and so ships
them in the feature's PR. That suits the Orchestrator repo. For other projects it
clutters the repository with a folder the team never asked for, and puts it in every PR.

The path is hard-coded (`FeatureCtx::docs_dir`, `team::feature_docs_dir`, the lead brief,
the protocol text, the `handoff` tool description, the Overwatch readers, the Notion
publisher). The existing `track_orchestrator_dir` flag only moves `policy.toml`; it has
no UI and does not touch feature docs.

## 2. Goal

Per project, the user chooses where a feature's **docs** and **designs** are saved:

- **In the repository, in a folder they pick** (one folder for both, or a separate folder
  for designs). Default: `.orchestrator/features/{slug}` with designs in its `design/`
  subfolder, so the Orchestrator project sees no change.
- **Not saved in the repository**: kept in the app's data, never committed, never in a PR.

Docs and designs are chosen independently (docs in `docs/specs/{slug}`, designs not
saved, for example).

## 3. Behaviour

### 3.1 The setting (Project settings → new section "Feature docs")

Two rows, **Docs** and **Designs**:

| Row | Options |
|---|---|
| Docs | *In the repository* + folder (template, default `.orchestrator/features/{slug}`) · *Not saved in the repository* |
| Designs | *With the docs* (default: `<docs folder>/design`) · *In the repository* + folder (template) · *Not saved in the repository* |

- A folder is a path relative to the repo root that contains `{slug}` (the feature's
  slug). Refused: absolute paths, `..`, a component `.git`, empty components, no
  `{slug}`, over 200 characters. The section shows the resolved example
  (`docs/specs/checkout/`) as the user types.
- *Not saved in the repository* keeps the files in
  `<app data>/projects/<project id>/features/<slug>/{docs,design}/`. The section says so in
  one line: "Kept on this Mac only, never committed. Deleted with the feature."
- *With the docs* while docs are not saved = designs not saved either (same app folder).
- Saved with the section's Save (⌘⏎), the same way as the other sections.

### 3.2 Snapshot at launch

The resolved locations are stored on the **feature when it is created**. Changing the
setting affects only features created afterwards; a running feature keeps its folders and
nothing is moved. Features created before this change (no stored location) use the legacy
`.orchestrator/features/{slug}` with `design/` inside, so they keep working.

### 3.3 In the repository (any folder)

Same as today, with the chosen path everywhere today's path appears: the lead brief and
protocol ("write the docs in `<folder>`, commit them"), the spec gate check and its
`doc_lines`, the Feature page, the Overwatch (briefing, chat, tools, grounding), Notion
publishing, the `handoff` tool description (designs under the designs folder), the
team-prompt line telling frontend where exported images are. Agents keep reading them
from their worktree after a sync.

### 3.4 Not saved in the repository

- Every agent of the run gets the feature's app folder(s) **read & write** in its sandbox
  and one prompt line naming them (like project files, FR-112). The hard-deny on writes
  outside the worktree lets these folders through; nothing else changes.
- The lead brief says: write the docs in `<absolute folder>`, do **not** commit them, then
  call `request_spec_approval`. The spec gate reads the docs from that folder; it still
  refuses a dirty worktree (code), not the docs.
- Designs: designers export straight into the app design folder and `handoff` that
  absolute path; `handoff` accepts an absolute path only when it lies inside the run's
  feature folder. Frontend reads them there, no sync needed for designs.
- The Feature page, gate preview, Overwatch and Notion read the same folder. Images a doc
  links (`design/home.png`, relative to the doc) resolve against the docs folder and must
  land in the docs or designs folder of the feature.
- Nothing of it reaches git: not committed, not in the PR.
- **Deleting the feature** deletes its app folder. Archiving keeps it.

### 3.5 Mixed choices

Docs and designs follow their own row: docs in the repo with designs not saved is valid
(the doc's image links won't resolve on GitHub — that is the user's choice). A designs
folder in the repo with docs not saved is valid too (designs are committed and merged as
today).

### 3.6 Feature page

Under the docs list, one line says where they live: "In the repository ·
`docs/specs/checkout/`" or "Not saved in the repository" with a **Reveal in Finder**
button (opens the app folder; the app opens it, not an agent).

## 4. Out of scope

- `policy.toml` and the `track_orchestrator_dir` flag (unchanged).
- The intake glob default `.orchestrator/backlog/*.md` (it is already a user setting).
- Moving docs of existing features when the setting changes.
- A custom folder outside the repo (Drive, Obsidian vault): only the app's data folder.
- Docs written by solo sessions.

## 5. Acceptance

1. A new project with no setting: a team run writes and commits
   `.orchestrator/features/<slug>/spec.md` exactly as today (existing tests unchanged).
2. Docs folder `docs/specs/{slug}`, designs *With the docs*: the lead is told
   `docs/specs/<slug>/`; the gate, Feature page, Overwatch and Notion read it; designs land
   in `docs/specs/<slug>/design/`.
3. Designs folder `docs/design/{slug}`: the `handoff` description and team prompt name it;
   a doc linking `../../design/<slug>/a.png` previews in the Feature page.
4. Docs *Not saved*: the lead writes in the app folder, the gate opens without a commit,
   the integration branch and the PR contain no doc; every agent can read and write the
   folder in its sandbox and no other folder outside its worktree; deleting the feature
   removes the folder.
5. Invalid folders (`/abs/{slug}`, `../x/{slug}`, `docs/specs`, `.git/{slug}`) are refused
   on save with a message on the field; nothing is saved.
6. Changing the setting while a feature runs leaves that feature's folders unchanged.
7. `docs/SPEC.md` FR-109, FR-504 and FR-1308 describe the new setting.
