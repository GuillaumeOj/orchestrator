# Add Auto Check for PR Status · Feature spec

> Feature `add-auto-check-pr-status` · Integration branch `orch/add-auto-check-pr-status/integration` · Core only (one store/runtime change, no UI code change, no design pass)

## 1. Problem

In the Feature pane, the **PR line** (`PR #n`, CI dot, position badge, `checked … ago`, ↻) only shows the real state of the pull request after the user presses ↻ (or switches away from the app and back). Left open, it keeps showing the state of the last manual refresh, and `checked 7 min ago` keeps growing.

**Goal:** the PR line follows the PR by itself: at most about a minute after GitHub changes (CI finishes, the base moves, the PR is merged or closed), the open Feature pane shows it, without any click.

## 2. Cause (HEAD bd5797c)

The core already checks every open PR every minute:

- `crates/agent-runtime/src/server.rs:55`: the `orch-fetch` thread calls `Runtime::pr_tick` every `PR_EVERY` (60 s, `crates/agent-runtime/src/pr.rs:25`).
- `pr_tick` → `pr_check` runs `gh pr view`, computes the status and stores it with `Store::set_run_pr` (`crates/orch-store/src/teams.rs:712`).

But `set_run_pr` writes with `Store::write`, not `Store::write_touching`: unlike every other team-table write (`upsert_run`, `upsert_task`, …) it notes **no `now` topic**. The UI only refetches the Feature pane on a topic (`FeaturePane.svelte`: `onTopics(['feature:<id>', 'features', 'teams'], load)`, "no poll"), on mount and on window focus. So the status the background check stores is never pushed to the open pane; only the manual `pr_refresh` (whose reply the PR line applies directly) and a window refocus show it.

Moves the check causes (merged → `done`, closed → `integrating`) already go through run-state writes that touch topics; only the status itself (CI, position, unpushed, `checked_at_ms`, `error`) is silent.

## 3. Scope

- `Store::set_run_pr` notes the `feature:<feature_id>` topic of the run once its write is committed (through `write_touching`, as the other team writes do). The feature id comes from the run row (e.g. `UPDATE … RETURNING feature_id`, or a lookup in the same transaction); the signature stays `set_run_pr(id, pr, url)` unless a caller-side change is simpler and keeps every call site correct.
- Only `feature:<id>`: the PR status is shown only in the Feature pane; lists (`features`, `teams`) are not refetched every minute. Whether it also counts as a team-data change (`touch`) or only a topic (`note`) follows what other run writes do (`touch`).
- Every stored check notes the topic, including one where nothing but `checked_at_ms` changed (so `checked just now` stays true) and a failed check (the line turns stale with `last checked …`).
- The interval stays `PR_EVERY` = 60 s; the offline / low-battery skip (FR-103), the 5-min base fetch throttle and the manual ↻ stay as they are.
- `crates/agent-runtime/README.md` (Open PR paragraph) and `crates/orch-store/README.md` (if it lists the team writes and their topics) say the PR status write raises `feature:<id>`.

No UI change: the Feature pane already refetches on `feature:<id>`, keeps the selected doc tab, the message draft and the CI popover across a refetch (props update in place), and `checked … ago` already re-renders every 30 s (`now30`).

Out of scope: a configurable interval, checking PRs more often while the pane is open, PR status anywhere outside the Feature pane, desktop notifications.

## 4. Acceptance

1. With a Feature pane open on a run in `pr_open`, a change on GitHub (CI turns green/red, base moves, PR merged) shows on the PR line within ~60 s with no click and the window focused; `checked` reads `just now` after each background check.
2. A background check that fails (gh error) turns the line stale (`last checked …`) without a click.
3. The manual ↻ works as before.
4. Store test (`crates/orch-store/src/teams.rs` tests): after `set_run_pr`, `take_topics()` contains `feature:<feature_id>` of that run (and not `features` / `teams`).
5. Runtime test (`crates/agent-runtime/src/pr.rs` or `now.rs` tests, with the `cfg.gh` override): a `pr_tick` on an open run produces a `now` event whose `topics` include `feature:<id>`.
6. `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` pass; UI tests untouched and green.

## 5. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 PR status writes raise the feature topic | back | `orch-store/src/teams.rs` (`set_run_pr` + test), runtime test, READMEs |

No frontend or design work (see `ux-brief.md` and `api-contract.md`).
