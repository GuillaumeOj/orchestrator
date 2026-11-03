# Add Build on PRs · UX brief

No design pass: one Settings section on the existing section pattern (Integrations, MCP servers), one status bar chip, two palette entries. `back` reviews the frontend task against this brief; follow `docs/design/tokens.json` and the existing Settings sections.

## Principles

- **You always know what you run.** A PR build is visibly a PR build, everywhere: status bar chip, Settings header.
- **Two clicks, no surprise.** Install downloads in the background; nothing restarts until you click Restart.
- **The way back is one click.** Back to main is always at the top while you run a non-main build.
- **Say what restarting does.** Agents are suspended and resume; your data is snapshotted first.

## Settings › Builds

New section in the Settings nav, after **Memory**: label **Builds**. Keys as the other sections (j/k rows, ⏎ action, `r` refresh, esc back to the nav).

### Header: the build you run

```
Running   0.1.57 · f9b1e20 · PR #12 Add build on PRs                [Back to main]
          main 0.1.61 is downloaded: Back to main restarts into it.
```

| Running | Line 1 | Line 2 | Action |
|---|---|---|---|
| main | `0.1.61 · a17c3d0 · main` | the main update state in words ("up to date", "0.1.62 ready, installs when you quit") | none |
| local | `0.1.0 · local build` | "main 0.1.61 is downloaded" / "checking main…" | **Back to main** |
| PR | `0.1.57 · f9b1e20 · PR #12 <title>` | as local | **Back to main** |

Back to main: when the main update is `ready`, it calls `update_restart({target: "main"})`; otherwise it is disabled with "main build downloading…" / the error, and a **Check** link runs `update_check`. Version and sha in mono; the sha links to the commit.

When updates are off (`enabled = false`), the section shows only the header and one muted line: "PR builds are off: <disabled_why>." (debug build, gh not logged in…).

### The list: open PRs

One row per open PR (from `pr_builds_list`), newest activity first. A muted line above: "Open pull requests · refreshed 14:02 · `r` refresh".

```
#14  Fix the week planner graph            build a17c3d0 · 12 min ago      [Install]
     fix-week-planner-graph · GuillaumeOj
#12  Add build on PRs                      ● running                         
#11  Improve agent pane                    built from an older commit       [Install]
     improve-agent-pane · GuillaumeOj      (newest commit not built yet)
#9   Draft: rework inbox                   draft: not built
#8   Bump tauri                            no build yet
```

| Row state | Right side | Action |
|---|---|---|
| `build.status = current` | `build <sha7> · <age>` | **Install** |
| `outdated` | "built from an older commit" + muted "(newest commit not built yet)" | **Install** (installs the older build) |
| no build, draft | "draft: not built" (muted) | none |
| no build | "no build yet" (muted; CI builds after the checks pass) | none |
| running (`current.channel = pr`, `current.pr = n`, same sha) | `● running` (accent dot) | none |
| running, but a newer build exists | `● running · newer build` | **Update** (= Install) |
| installing: `pr_install.pr = n`, `downloading` | `Downloading · 40 %` | none (other rows' Install disabled) |
| `pr_install.pr = n`, `ready` | `Ready` | **Restart to run** (accent) |
| `pr_install.pr = n`, `error` | the error, amber | **Retry** |

Row title and PR number link to the PR on GitHub (opens the browser). Branch and author on the second line, muted.

**Restart to run** asks once, inline in the row (not a modal): "Restart into PR #14? Agents are suspended and resume. Your data is snapshotted first; Back to main restores it if this PR changes the database. **Restart** · Cancel". From a PR build to another PR build the second sentence becomes "Your data is not snapshotted again (the snapshot from before PR #12 is kept)."

Empty and errors: no open PR → "No open pull requests." `error` → the error text, amber, with **Retry**. Loading → the previous list stays, the refreshed line says "refreshing…".

## Status bar

- While running a PR build: chip `PR #12` on the right, before the update chip, accent border (like the ready update chip). Click: opens Settings › Builds. Tooltip: "You run PR #12's build (f9b1e20): <title>. Click to change build or go back to main."
- While running a local build: no new chip (unchanged).
- The main update chip, when running a PR or local build: tooltip "Back to main 0.1.61: restarts into it (agents are suspended and resume)."

## Palette

- **Install a PR build…**: opens Settings › Builds.
- **Back to main**: enabled when running a `pr` or `local` build and the main update is `ready`; runs `update_restart({target: "main"})`.
- "About Orchestrator" (existing) adds `PR #12` when running a PR build.

## Toasts

- When `pr_install` becomes `ready`: "PR #14's build is ready. **Restart to run**" (asks the same inline confirm in Settings, so the action opens Settings › Builds on that row).
- A start-up warning after a restore (api-contract §6): shown as an amber toast, long duration (15 s), with the text as given.

## Copy rules

Versions and shas in mono, sha 7 chars. "PR #12", never "pull request 12" in rows. No "prerelease", "artifact", "channel" in the UI.
