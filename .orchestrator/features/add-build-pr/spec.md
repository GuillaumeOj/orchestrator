# Add Build on PRs · Feature spec

> Feature `add-build-pr` · Integration branch `orch/add-build-pr/integration` · CI + core (Rust) + one Settings section, no design pass

## 1. Goal

Every open pull request gets its own signed `Orchestrator.app`, built by CI. From the app (Settings › **Builds**) you pick an open PR, install its build in place of the one you run, test it on your real data, and go back to `main` with one click. No more checking out the branch, `make install`, and fighting the updater over which bundle is in `~/Applications`.

## 2. What exists today (add-build-on-merge-to-main)

- CI job `app` builds every green `main` commit and publishes release `v0.1.<run_number>` (`Orchestrator_0.1.<n>_aarch64.app.tar.gz` + minisign `.sig`), keeping the 10 newest.
- `crates/orch-app/src/update.rs`: finds the latest release with `gh`, downloads it, verifies the signature against `update.pub`, stages it under `<app data>/updates/<version>/`, swaps the bundle on quit (channel `main`) or on "Restart to update" (any channel).
- Build info baked at compile time: version, sha, channel (`main` / `local`).
- The store refuses to open a database whose schema is newer than the build (`migrations.rs`); the app then runs on a scratch store ("projects and settings are not kept").

Today, to test a PR you build it yourself (channel `local`), and the status bar keeps offering to replace it with `main`. And if the PR added a migration, going back to `main` leaves you on a scratch store.

## 3. How it works, end to end

```
push to a PR branch → CI (fmt, rust, ui) green
                        └→ job `app-pr` (macOS): build, sign, publish prerelease `pr-<n>` (replaces the previous one)
PR closed / merged   → workflow `pr-build-cleanup`: delete release + tag `pr-<n>`

Settings › Builds → open PRs (gh) × their `pr-<n>` builds → Install #12
   → download + verify signature → staged → "Restart to run PR #12"
   → snapshot of the database → quit as ⌘Q (agents suspended) → swap → relaunch as PR #12's build
Back to main → the main updater's staged build → Restart → main build again
   (the database is restored from the snapshot only if the PR build left it on a newer schema)
```

## 4. Scope

### 4.1 CI: build every PR (back, T-1)

- New job `app-pr` in `.github/workflows/ci.yml`, beside `app`:
  - `needs: [rust-fmt, rust, ui]` (a red PR is not built).
  - `if: github.event_name == 'pull_request' && github.event.pull_request.head.repo.full_name == github.repository && !github.event.pull_request.draft && github.event.pull_request.user.login != 'renovate[bot]'`. Fork PRs never get the signing secret, so they are never built; drafts and Renovate PRs are skipped to save macOS minutes (§7).
  - `concurrency: { group: app-pr-${{ github.event.pull_request.number }}, cancel-in-progress: true }`: a newer push supersedes the build in progress.
  - `permissions: contents: write` (job level; workflow default stays `read`).
  - Same steps as `app` (toolchain, rust-cache, bun, font, `scripts/app.sh build`, `scripts/app.sh package`), with `ORCH_VERSION=0.1.<run_number>`, `ORCH_BUILD_SHA=<PR head sha>` (`github.event.pull_request.head.sha`, not the merge commit), `ORCH_BUILD_CHANNEL=pr`, `ORCH_BUILD_PR=<number>`. Checkout the **head** sha (`ref: ${{ github.event.pull_request.head.sha }}`) so the build is exactly the PR's commit.
  - Publish: delete the release `pr-<n>` and its tag if they exist, then `gh release create pr-<n> --target <head sha> --prerelease --latest=false --title "PR #<n> · <title>"` with the tarball and `.sig`. Body format in [api-contract.md](api-contract.md) §1. A prerelease is never "latest", so the main updater never sees it.
  - Shared steps between `app` and `app-pr` go into a composite action or stay duplicated; `scripts/app.sh` stays the one build path. Owner's choice, keep the diff readable.
- Add `ready_for_review` to the `pull_request` trigger types (with the default `opened, synchronize, reopened`) so a draft is built when it becomes ready.
- New workflow `.github/workflows/pr-build-cleanup.yml`: `on: pull_request: types: [closed]`, deletes release `pr-<n>` with `--cleanup-tag` (no-op if absent). `permissions: contents: write`. Runs on `ubuntu-latest`.
- The main `app` job's prune step keeps matching `^v0\.1\.[0-9]+$` only (PR releases are untouched by it).
- `scripts/app.sh`: passes `ORCH_BUILD_PR` through like the other `ORCH_BUILD_*`.
- `orch-app` build info gains `pr: Option<u64>` from `ORCH_BUILD_PR` (`option_env!`), shown in the start log line.

### 4.2 The app installs a PR build (back, T-2)

In `crates/orch-app/src/update.rs` (or a new `pr_builds.rs` beside it that reuses its download / verify / stage / swap helpers; owner's choice).

- **List** (`pr_builds_list`): two `gh` calls, run when the Settings section opens and on refresh, never in the background:
  - `gh pr list --repo GuillaumeOj/orchestrator --state open --limit 50 --json number,title,headRefName,headRefOid,author,isDraft,updatedAt,url`;
  - `gh api repos/GuillaumeOj/orchestrator/releases?per_page=100` filtered to prereleases tagged `pr-<n>` (tag, target sha, assets, published time).
  - Joined per PR: build `current` (built from the head sha), `outdated` (built from an older commit of the PR), or `none`. Format: api-contract §4.
- **Install** (`pr_build_install {number}`): downloads that PR's two assets into `updates/download/`, verifies the signature with the same key and the same rules as main updates (trusted comment names the file), extracts it to `updates/pr-<n>/Orchestrator.app` with a `release.json` (`version`, `sha`, `pr`, `title`). One PR build staged at most (a new install replaces the previous one); it is kept apart from the staged main update, which stays.
  - Refused while the updater is off (same `disabled_why` rules: debug build, `ORCH_NO_UPDATE`, bundle not writable, `target/`, no key, gh missing / logged out).
  - One install at a time; a second call while downloading is refused.
  - Progress and outcome go through the existing `update` event (field `pr_install`, api-contract §3).
- **Restart** (`update_restart {target}`): `target` is `"main"` (the default, today's behaviour) or `"pr"`. For `pr`: the database snapshot (§4.3) is taken, then quit as ⌘Q does (agents suspended, resumed by session id next start), swap, relaunch, exactly as a main update.
- **A PR build is never applied on a plain quit**; only by its Restart.
- **Running a PR build** (channel `pr`): it behaves like a local build toward main updates: the main updater still checks and stages the latest `main`, never applies it on quit, and "Restart" with target `main` is the way back. That is **Back to main** in Settings.
- Leftovers: the start-up cleanup also removes a `pr-<n>` staging dir for a PR that is the running build (already installed) or no longer staged.

### 4.3 Your data survives a PR with a migration (back, T-3)

A PR build may add a migration; once it ran, a `main` build refuses the database. So:

- **Snapshot**: when a restart into a PR build starts **from a `main` or `local` build**, the core writes a consistent copy of the store (`VACUUM INTO`, or the SQLite backup API, through `orch-store`) to `<app data>/state/backup/orch.db` with a `backup.json` (`schema`, `taken_at`, `from`: build info, `to_pr`). The blobs dir (`orch.blobs/`) is copied too (hard links when possible). Switching from one PR build to another keeps the existing snapshot (it is the last state a release build could read).
- **Restore**: at start, when `Store::open` fails because the schema is newer than this build, and a snapshot exists whose `schema` ≤ this build's `SCHEMA_VERSION`:
  1. the current `orch.db` (+ `-wal`, `-shm`, blobs) moves to `state/aside/orch-<taken_at>-schema-<v>/` (kept, never deleted by the app);
  2. the snapshot is copied in place and opened normally (pending migrations run);
  3. the app shows a warning (existing `warnings` channel, a toast in the UI): "Your data was restored from before PR #12 (taken 14:02). What changed while testing it is kept in state/aside/…".
  Without a usable snapshot the behaviour is today's (scratch store + warning).
- The snapshot is removed when a `main` build opens the database normally and the snapshot is older than 14 days, or replaced by the next one.
- Nothing else in `<app data>` is touched (agents dirs, bin, logs): helpers are re-copied from the bundle at each start as today.

### 4.4 Settings › Builds (front, T-4)

See [ux-brief.md](ux-brief.md). In short:

- New Settings section **Builds**: the build you run (version, short sha, channel, PR), Back to main, then the open PRs with their build state and an Install / Restart action per row.
- Status bar: while you run a PR build, a persistent chip `PR #12` (opens Settings › Builds), so you never forget you are on a test build. The main update chip's tooltip says it brings you back to main.
- Palette: "Install a PR build…" (opens Settings › Builds), "Back to main" (enabled when running a non-main build).

### 4.5 Docs

- `docs/app/RUNNING.md` › Updates: PR builds (when they are built, where they live, how to install one, going back, the database snapshot and `state/aside/`, Keychain prompts).
- `docs/app/RUNNING.md` › Where data lives: `updates/pr-<n>/`, `state/backup/`, `state/aside/`.

## 5. Out of scope

- Fork PRs (no signing secret; never built).
- Running a PR build **beside** the main one (a second bundle id, a separate data dir): one app, swapped in place.
- Merging the app's data back from a PR build (what changed while testing stays in `state/aside/`).
- Building on demand from the app (CI builds every push; there is no "build this PR now" button).
- CI run status in the list (building / failed): the list shows the last published build only.
- Intel / universal builds, notarization (unchanged).

## 6. Acceptance

1. A push to a same-repo, non-draft PR with green CI publishes prerelease `pr-<n>` with the tarball and `.sig`, target = the head sha; a second push replaces it; the main `app` job and the main updater ignore it; `gh release view` (latest) still returns the newest `v0.1.*`.
2. Closing or merging the PR deletes `pr-<n>` and its tag.
3. A fork PR, a draft PR and a Renovate PR do not run `app-pr`.
4. Core unit tests: PR list ⨝ releases (`current` / `outdated` / `none`, parsing fixtures of both gh outputs), install verify accepted / rejected (fixture key), one install at a time, refused when off, restart target `pr` needs a staged PR build, a PR build is never applied on quit, running channel `pr` sees any main release as newer, snapshot written on main→PR and kept on PR→PR, restore on a newer schema (temp dirs: aside dir kept, snapshot opened, warning emitted), no restore when the snapshot's schema is newer too.
5. UI tests: Settings › Builds rows for each state (none, outdated, current, running, downloading, ready, error), Back to main enabled state, the `PR #n` status chip.
6. Manual check, written in the final report: from a `main` build, install this feature's own PR build from Settings, restart, see `PR #<n>` in the status bar with agents resumed, then Back to main and see the main build with the same data.
7. `make ci` passes.

## 7. Open questions for you

- **macOS minutes**: every push to every non-draft PR builds the app (≈ 15–25 min on `macos-latest`, after the tests). Free on a public repo; on a private repo macOS minutes count 10×. The spec skips drafts and Renovate PRs. Fine, or build only PRs with a label (e.g. `build`)?
- **Restoring data**: going back to main after a PR that changed the database restores the snapshot automatically (what you did while testing is kept aside, not merged). Fine, or should the app ask first?

## 8. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 CI: build and publish PR builds | back | `ci.yml` job `app-pr` + trigger types, `pr-build-cleanup.yml`, `scripts/app.sh` (`ORCH_BUILD_PR`), build info `pr`, RUNNING.md |
| T-2 Install a PR build | back | `update.rs` (or `pr_builds.rs`), commands `pr_builds_list`, `pr_build_install`, `update_restart {target}`, `update` state `pr_install` + `current.pr`, fixtures, tests. Plan first. Depends on T-1 |
| T-3 Database snapshot and restore | back | `orch-store` (backup), `core.rs` `open_store`, the snapshot hook on restart into a PR build, tests. Depends on T-2 |
| T-4 Settings › Builds | front | `SettingsPane.svelte` section, new `BuildsSettings.svelte`, `lib/update.js`, `StatusBar.svelte` PR chip, palette entries. Depends on T-2; `back` reviews |
