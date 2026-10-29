# Improve the end of a feature · UX brief

> Companion of [spec.md](spec.md) and [api-contract.md](api-contract.md). Small pass: one line in an existing card, one new run state, one gate card. UX settles states, copy and behaviour; UI draws them in Penpot (04 Feature · 1512, card "PR and artifacts") with the existing tokens and exports the frames under the feature's `design/` folder.

## 1. The PR line (Feature pane, card "PR and artifacts")

Replaces `PR https://github.com/…/pull/12`. One line, no wrap at the card's minimum width:

`PR #12` · `[CI]` · `[position]` · `[↑2 to push]` · `checked 2 min ago` · `[refresh]`

- **`PR #12`**: a link (opens the PR in the browser); tooltip = full URL.
- **CI badge** (icon + short word, status colors from tokens): `passing` · `failing` (with count when > 1: `2 failing`) · `pending` · `no checks`. Tooltip: the failing checks, each a link; else the counts.
- **Position badge**: `up to date` · `↓3 behind main` · `conflicts` · `—` (unknown, while GitHub computes). Conflicts uses the error color, behind the warning one, up to date is quiet.
- **`↑2 to push`**: only when `unpushed > 0`. Quiet; it tells the user a push gate is coming or pending.
- **Merged / closed**: when `pr.state` is not `open`, the CI and position badges give way to one badge `merged` (done color) or `closed` (muted).
- **Stale**: when `error` is set, the line dims and the "checked" text reads `last checked 12 min ago · <error>` (tooltip).
- **Before the first check** (`pr` null, `pr_number` set): `PR #12` + `checking…`.
- **No PR**: the current text stays.
- Refresh: a small icon button; spins while the call runs; the toast states the error if any.

## 2. Run state "PR open"

New state between "PR gate" and "Done" everywhere a run state shows (sidebar, palette, Feature pane header, timeline). Color: an active state, distinct from "PR gate" (at gate) and "Done". Done now means merged.

## 3. Update gate card (Inbox)

Gate `push_pr_update`: title `push 2 commits to PR #12`; body: the commit subjects (short sha + subject, up to 10, then "+N more"), the PR link. Approve / Reject with message, like the PR gate.

## 4. Timeline copy

- `pr.opened`: `opened PR #12`
- `pr.status`: `PR #12 CI failing (ci / test)` · `PR #12 CI passing` · `PR #12 behind main by 3` · `PR #12 conflicts with main` · `PR #12 up to date`
- `pr.pushed`: `pushed 2 commits to PR #12`
- `pr.merged`: `PR #12 merged`
- `pr.closed`: `PR #12 closed without merge`

## 5. States to draw
PR line: checking · open/passing/up to date · pending · failing (1 and several, tooltip open) · behind · conflicts · ↑ to push · stale · merged · closed. Run state badge "PR open". The update gate card.
