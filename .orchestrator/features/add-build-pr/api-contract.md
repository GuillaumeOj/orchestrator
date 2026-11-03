# Add Build on PRs · API contract

Between CI (T-1), the core (T-2, T-3) and the UI (T-4). Extends [add-build-on-merge-to-main's contract](../add-build-on-merge-to-main/api-contract.md); everything there stays valid.

## 1. PR release (T-1 → T-2)

On `GuillaumeOj/orchestrator`, one per open PR:

| Field | Value |
|---|---|
| tag | `pr-<number>` |
| prerelease | `true` (never "latest") |
| target_commitish | the PR head sha (full) |
| name | `PR #<number> · <title>` |
| body | line 1: the PR title; then `Commit: <head sha>`, `Version: 0.1.<run_number>`, `Run: <run url>` |
| assets | exactly `Orchestrator_0.1.<run>_aarch64.app.tar.gz` and its `.sig` (same format as main, api-contract §1 of the main feature) |

The version is read from the tarball's name (`Orchestrator_<version>_aarch64.app.tar.gz`), never from the tag. A push replaces the release (delete + create), so the tag always points at the newest built commit.

## 2. Build info (T-1)

`BuildInfo` gains `pr`:

```json
{ "version": "0.1.57", "sha": "f9b1e20…", "channel": "pr", "pr": 12 }
```

`channel`: `"main"`, `"local"` or `"pr"`. `pr` is a number when `channel = "pr"`, else `null`. From `ORCH_BUILD_PR` (`option_env!`).

## 3. `update` state additions (T-2)

The `update` object (in `app_state` and the `update` event) gains one field; `current` gains `pr` (§2).

```json
{
  "current": { "version": "0.1.57", "sha": "f9b1e20…", "channel": "pr", "pr": 12 },
  "…": "all the existing fields, unchanged",
  "pr_install": {
    "pr": 14,
    "title": "Fix the week planner graph",
    "version": "0.1.61",
    "sha": "a17c3d0…",
    "state": "ready",
    "progress": null,
    "error": null
  }
}
```

| Field | Values |
|---|---|
| `pr_install` | `null` when no PR build is being installed or staged |
| `pr_install.state` | `"downloading"`, `"ready"` (verified and staged), `"error"` |
| `pr_install.progress` | `0.0..1.0` while downloading when known, else `null` |
| `pr_install.error` | short text when `state = "error"` (`"signature does not match"`, `"offline"`, `"gh: …"`, `"no build for PR #14"`), else `null` |

A staged PR build survives an app restart (read back from `updates/pr-<n>/release.json`), unless it is the build now running.

Main-update fields for a `pr` build behave as for `local`: any main release is newer, `apply_on_quit` is `false`.

## 4. `pr_builds_list` (T-2)

No args. Returns (it runs `gh`; up to a few seconds):

```json
{
  "fetched_at": 1793600123,
  "error": null,
  "prs": [
    {
      "number": 12,
      "title": "Add build on PRs",
      "branch": "orch/add-build-pr/integration",
      "author": "GuillaumeOj",
      "url": "https://github.com/GuillaumeOj/orchestrator/pull/12",
      "draft": false,
      "head_sha": "f9b1e20…",
      "updated_at": 1793599000,
      "build": {
        "status": "current",
        "version": "0.1.57",
        "sha": "f9b1e20…",
        "published_at": 1793599500
      }
    }
  ]
}
```

| Field | Values |
|---|---|
| `error` | `null`, or why the list is empty: `"offline"`, `"gh not installed"`, `"gh not logged in"`, `"gh: …"`; `prs` is `[]` then |
| `prs` | open PRs, newest `updated_at` first |
| `build` | `null` when there is no `pr-<n>` release (or it has not both assets) |
| `build.status` | `"current"` (`build.sha == head_sha`) or `"outdated"` (built from an older commit) |

Times are Unix seconds. Whether a row is the running build, or the one being installed, the UI derives from `update.current` (`channel = "pr"`, `pr`, `sha`) and `update.pr_install`.

## 5. Commands

| Command | Args | Returns | Effect |
|---|---|---|---|
| `pr_builds_list` | none | §4 | Lists open PRs and their builds. Never `Err` for gh / network trouble (§4 `error`); `Err` only when updates are off (`disabled_why`) |
| `pr_build_install` | `{ number }` | the `update` object | Starts downloading PR `number`'s build (progress in `pr_install` through `update` events). `Err` when updates are off, an install is already downloading, or the PR has no build. Installing the build already running is refused (`"already running"`) |
| `update_restart` | `{ target }`, optional, `"main"` (default) or `"pr"` | never returns on success | `main`: today's behaviour. `pr`: needs `pr_install.state = "ready"` (else `Err("no PR build ready")`); takes the database snapshot when the running build is `main` or `local` (T-3), then quits as ⌘Q does, swaps, relaunches. A failed swap keeps the app up with `pr_install.state = "error"` |

## 6. Database snapshot (T-3)

Not seen by the UI except through the existing start-up `warnings` (shown as a toast): after a restore, one warning `"Your data was restored from before PR #<n> (taken <hh:mm, date>). What changed while testing it is kept in <path>."`.

Files under `<app data>/state/`:

| Path | What |
|---|---|
| `backup/orch.db`, `backup/orch.blobs/`, `backup/backup.json` | The snapshot. `backup.json`: `{ "schema": 11, "taken_at": 1793600000, "from": <BuildInfo>, "to_pr": 12 }` |
| `aside/orch-<taken_at>-schema-<v>/` | A database moved aside by a restore. Never removed by the app |
