# Improve the end of a feature · UX brief

> Companion of [spec.md](spec.md) and [api-contract.md](api-contract.md). Small pass: one line in an existing card, one new run state, one gate card. UX settles states, copy and behaviour; UI draws them in Penpot (04 Feature · 1512, card "PR and artifacts") with the existing tokens and exports the frames under the feature's `design/` folder.

## 1. The PR line (Feature pane, card "PR and artifacts")

Replaces `PR https://github.com/…/pull/12`. One line, no wrap at the card's minimum width:

`PR #12` · `[CI]` · `[position]` · `[↑2 to push]` · `checked 2 min ago` · `[refresh]`

- **`PR #12`**: a link (opens the PR in the browser); tooltip = full URL.
- **CI**: a **filled circle** (no other icon) + **`done/total`** jobs, e.g. `● 4/6`. Circle colors (status tokens): **green** all finished jobs ok · **red** at least one job failed, errored or was cancelled · **yellow** jobs running · **grey** jobs waiting to start (queued, pending). `done` counts every finished job, whatever its outcome (ok, failed, cancelled, skipped). No checks: grey circle + `no checks`.
- **CI tooltip**: the full job list, one row per job: its own small circle (same color rule, per job), the job name (workflow as a muted prefix when there is one), its state word (`passed`, `failed`, `cancelled`, `skipped`, `running`, `queued`, `waiting`…), each row a link to the job. Order: failed first, then running, waiting, passed/skipped. Scrolls past ~12 rows.
- **Position badge**: `up to date` · `↓3 behind main` · `conflicts` · `—` (unknown, while GitHub computes). Conflicts uses the error color, behind the warning one, up to date is quiet.
- **`↑2 to push`**: only when `unpushed > 0`. Quiet; it tells the user a push gate is coming or pending.
- **`rebased`**: when `rewritten` (the branch was rebased onto main and not pushed yet): a quiet warning-colored tag, tooltip "the next push replaces the PR's history (force-with-lease)".
- **Merged / closed**: when `pr.state` is not `open`, the CI and position badges give way to one badge `merged` (done color) or `closed` (muted).
- **Stale**: when `error` is set, the line dims and the "checked" text reads `last checked 12 min ago · <error>` (tooltip).
- **Before the first check** (`pr` null, `pr_number` set): `PR #12` + `checking…`.
- **No PR**: the current text stays.
- Refresh: a small icon button; spins while the call runs; the toast states the error if any.

## 2. Run state "PR open"

New state between "PR gate" and "Done" everywhere a run state shows (sidebar, palette, Feature pane header, timeline). Color: an active state, distinct from "PR gate" (at gate) and "Done". Done now means merged.

## 3. Update gate card (Inbox)

Gate `push_pr_update`: title `push 2 commits to PR #12`, or `force-push PR #12 rebased onto main` when `force`; body: the commit subjects (short sha + subject, up to 10, then "+N more"), the PR link, and when `force` a warning line "Rewrites the PR's history: rebased onto main. Pushed with force-with-lease: refused if someone else pushed to the PR meanwhile." Approve / Reject with message, like the PR gate.

## 4. Timeline copy

- `pr.opened`: `opened PR #12`
- `pr.status`: `PR #12 CI failed (ci / test)` · `PR #12 CI ok` · `PR #12 behind main by 3` · `PR #12 conflicts with main` · `PR #12 up to date`
- `pr.pushed`: `pushed 2 commits to PR #12` · `force-pushed PR #12 (rebased)`
- `git.rebase`: `rebased the feature onto main (3 new commits from main)`
- `pr.merged`: `PR #12 merged`
- `pr.closed`: `PR #12 closed without merge`

## 5. States to draw
PR line: checking · CI green `6/6` · yellow `2/6` (running) · grey `0/6` (waiting) · red `5/6` (tooltip open with the full job list) · no checks · behind · conflicts · ↑ to push · rebased · stale · merged · closed. Update gate card: normal and force-push. Run state badge "PR open". The update gate card.
