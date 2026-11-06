# Add External Files To Project: spec

## Problem

An agent sees its own worktree and nothing else of the disk it may write to.
Its sandbox (`crates/agent-runtime/src/compile.rs`) allows writes in the
worktree (and the git dir) only, and its Read/Edit permission rules allow
nothing outside the worktree, so a file outside it prompts or is refused.

Projects often depend on material that lives outside the git repository:

- a brand guide: a folder of Markdown, PDFs and images kept elsewhere;
- a design file (a `.figcraft`, `.pen`, Figma export…) the team works on
  but that is not committed;
- notes, glossaries, API dumps shared by several repos.

Today the user has to copy them into the repo (and keep two copies in sync)
or paste their paths into every brief, and agents cannot change them anyway.

## Goal

The user attaches files and folders to a **project**, from anywhere on the
disk. Every agent of that project (solo sessions and team nodes) is told
where they are and what they are for, can read them without a prompt, and can
change them when the user allowed it.

## Scope

In:

1. **Project files**: a list per project, stored in the project settings.
   Each entry is one absolute path (a file or a folder, folder = everything
   under it) with:
   - **access**: `write` (read & write, default: the brief asks for files
     agents update) or `read` (read only);
   - **note**: optional free text, "what it is for" (e.g. "Brand guide: logo,
     colors, tone"). Agents see it.
2. **Add** one or several paths at once: native file/folder picker
   (multi-select, files and folders), drag and drop from Finder onto the
   section, or typed/pasted paths (`~` expanded, one per line).
3. **Edit** access and note, **remove** an entry (the files on disk are never
   touched), **reveal** in Finder, **open** in the editor (FR-1101) for files.
4. **Agents get them at spawn** (first start and every resume, like MCP
   registry refs, FR-601):
   - sandbox (Bash): `write` entries in `sandbox.filesystem.allowWrite`;
   - permission rules: `Read(//p/**)` (folder) or `Read(//p)` (file) allowed
     for every entry; `Edit(...)` allowed for `write` entries, denied for
     `read` entries;
   - folders also passed with `--add-dir` so the Read/Glob/Grep tools treat
     them as working directories;
   - a **Project files** section appended to the system prompt
     (`--append-system-prompt`, after the role prompt), see Behaviour.
5. **Project settings › Files**: a new section in the project settings pane
   (`ui/src/panes/ProjectSettingsPane.svelte`) to do all of the above.
6. Design: UX brief, then the UI frame next to the other Project settings
   boards in FigCraft, exports in `design/` of this feature.
7. Docs: `docs/SPEC.md` (new FR-112 under Projects; FR-109 lists the new
   setting), `crates/agent-runtime/README.md` (what a spawn adds for project
   files), ADR-009 (the sandbox gains user-approved paths, consequences).

Out:

- Copying, syncing or versioning the files. They are shared live by every
  agent of the project, not isolated per worktree and not in git: no history,
  no merge, no review (see Risks).
- Agents adding or removing entries themselves (only the user, in the app).
- Files shared across all projects (a global list); per project only.
- Applying a change to a running agent's process: a sandbox is fixed when the
  process starts (see Behaviour).
- The Overwatch: it belongs to no project.
- Watching the files for changes (FR-1103 covers worktrees only).

## Behaviour

### Stored

`ProjectSettings.external_files` (`crates/orch-store/src/teams.rs`), a list
of `{path, kind: "file"|"folder", access: "read"|"write", note?,
added_at_ms}`. The path is stored canonical (`~` expanded, symlinks
resolved). Order = the order added. Kept by the dedicated commands only:
`project_settings_set` (the General/backend form) keeps the stored list as
is, whatever its form carries, so the two editors never erase each other.

### Validation (adding, core side)

A path is refused, with a message for that path, when:

| case | message |
|---|---|
| not absolute after `~` expansion | `{p}: use an absolute path or ~/…` |
| missing | `{p}: no such file or folder` |
| already listed (same canonical path) | `{p}: already in the project files` |
| inside, or equal to, the project's main checkout or its worktree root | `{p}: already part of the project (in its repository)` |
| `/`, the home folder, or a folder that contains the home folder | `{p}: too broad, pick a folder inside it` |
| inside or containing the app's data folder | `{p}: the app's own data can't be shared with agents` |
| inside or containing a credential store (`CREDENTIAL_STORES`: `~/.ssh`, `~/.aws`, Keychains…) | `{p}: holds credentials, can't be shared with agents` |
| containing the project's main checkout or another project's | `{p}: contains a project repository, pick a folder inside it` |

A folder that is inside another listed folder is accepted (it can carry its
own access and note); the narrower entry wins for its subtree (a `read`
folder inside a `write` folder: its files are read only). At most 100
entries per project.

Adding several paths is all-or-some: valid paths are added, refused ones are
returned with their message; the UI shows both.

### At spawn

`Runtime::spawn` reads the agent's project's `external_files` and passes them
to `compile` (new `CompileCtx::project_files`). For each entry that exists at
spawn (re-canonicalized, re-validated against the rules above; one that now
fails is skipped and logged):

- `read`: `Read(//p/**)` / `Read(//p)` in `permissions.allow`, and
  `Edit(//p/**)` / `Edit(//p)` in `permissions.deny` (no Bash write either:
  not in `allowWrite`).
- `write`: the Read and Edit rules in `permissions.allow`, and the path in
  `sandbox.filesystem.allowWrite`.
- folder: `--add-dir <p>`.

The generated denies (credential stores, main checkout, other worktrees, app
data) stay and win, which the validation keeps from overlapping anyway.

Missing at spawn: skipped from the settings and listed in the prompt as
missing, so the agent can say so instead of searching for it.

The prompt section (omitted when the list is empty):

```
## Project files
The user attached these to the project. They live outside the git repository: they are shared live by every agent of the project and are not versioned. Read them when your task needs them. Before changing one, re-read it and change only what your task needs; never delete or move them. Commit nothing of them.
- `/Users/me/Brand` (folder, read & write): Brand guide: logo, colors, tone
- `/Users/me/design/app.figcraft` (file, read & write)
- `/Users/me/specs/api.yaml` (file, read only): Partner API
- `/Users/me/old.pdf` (file, missing)
```

### Changes while agents run

Saving the list applies at each agent's next spawn (start, resume after a
suspend or a restart). Running processes keep the files they started with.
The section says so ("Running agents get changes when they next start") and
names how many agents of the project are live. No automatic restart.

### Audit

`project_file_added` `{path, kind, access}`, `project_file_changed`
`{path, access?, note_changed}`, `project_file_removed` `{path}`, by `user`,
on the project.

## Risks

- **Shared, unversioned writes**: two agents (or an agent and the user) may
  write the same file; the last write wins and nothing can undo it. Mitigated
  by `read` access per entry, the prompt guidance, and the UI copy on `write`.
  Snapshots or versioning are out of scope (a later feature if needed).
- **Wider sandbox**: each entry widens what agents may read and write, and
  what they may send to the model. Only the user adds entries, in the app;
  the validation refuses credentials, the app's data, the home folder and
  broader; every change is audited (ADR-009 updated).
- **Single files and Bash**: the sandbox allows writing a listed file, but a
  tool that writes through a temp file and a rename in the same folder fails
  under the sandbox. The Edit/Write tools and MCP servers (e.g. FigCraft) are
  not affected. The UI suggests adding the folder when a Bash tool must write.

## Acceptance

1. In Project settings › Files, the user adds a folder and two files at once
   (picker multi-select, or Finder drag and drop); they are listed with
   kind, access Read & write, no note. A typed `~/x` path is expanded.
2. `~/.ssh`, `~`, the project's own `src/`, and a missing path are refused
   with their message; valid paths in the same batch are added.
3. A new solo agent of the project: its `settings.json` has the rules and
   `allowWrite` above, its command line has `--add-dir` for the folder, its
   system prompt has the Project files section with the notes. It reads and
   edits a file of the folder without a permission prompt, from the Edit tool
   and from Bash.
4. Switching an entry to Read only, then resuming the agent: Edit on it is
   denied, Bash can't write it, Read still works.
5. Team nodes of a feature of the project get the same files and section.
6. Removing an entry leaves the files on disk; the next spawn no longer has
   it. Saving the General form does not erase the list.
7. An entry whose path disappeared shows as missing in the section and in the
   prompt, and the spawn still succeeds.
8. `cargo test` and the UI tests pass; new tests cover validation (each
   refusal), the compile output (read/write, file/folder, missing), the
   prompt section, `project_settings_set` keeping the list, and the UI
   section (add batch with refusals, access toggle, remove).
