# Improve the end of a feature · UX brief

> Companion of [spec.md](spec.md) and [api-contract.md](api-contract.md). Small pass: one line in an existing card, one new run state, one gate card. UX settles states, copy and behaviour; UI draws them in Penpot (04 Feature · 1512, card "PR and artifacts", and the Inbox detail) with the existing tokens and exports the frames under the feature's `design/` folder. Wireframes (low-fi, layout and copy only): [design/ux-pr-line.svg](design/ux-pr-line.svg), [design/ux-update-gate.svg](design/ux-update-gate.svg).
>
> Revision 2 (T-47): final brief. Every state with its copy, the CI dot rule as the user decided it, the tooltip, truncation, the "PR open" color, the update gate card, keyboard and accessibility.

## 0. Tokens used

No new token. Everything comes from `docs/design/tokens.json` (`color.status.*`, `color.text.muted`, `size.status-dot` = 8, `space.*`, `text.caption`, `text.label`).

| Role in this brief | Token | Dark / light |
|---|---|---|
| **ok** (CI green, `merged`) | `color.status.done` | green / green-aa |
| **error** (CI red, `conflicts`, force-push warning) | `color.status.error` | red / red-aa |
| **running** (CI yellow) | `color.status.queued` | yellow / yellow-aa |
| **waiting** (CI grey, no checks) | `color.status.idle` | dark5-aa / dark5-aa |
| **attention** (`behind`, `rebased`) | `color.status.waiting-for-you` | orange / orange-aa |
| **quiet** (`up to date`, `↑2 to push`, `checked …`, `closed`) | `color.text.muted` | |
| **PR open** run state | `color.status.working` | cyan / cyan-aa (see §2) |

There is no "warning" token: the first brief's "warning color" is `status.waiting-for-you` (orange). It stays distinct from the CI yellow, so "behind" never reads as "CI running".

## 1. The PR line (Feature pane, card "PR and artifacts")

Replaces `PR https://github.com/…/pull/12` (`FeaturePane.svelte`, `.arow` under "PR and artifacts"). **One line, never wraps.** Items left to right, separated by `space.sm` (no `·` glyph between items: each item is its own chip or text):

```
PR #12   ● 4/6   ↓3 behind main   ↑2 to push   rebased            checked 2m ago  ⟳
└link┘   └CI┘    └position┘       └unpushed┘   └rewritten┘ (grow) └checked┘       └refresh┘
```

`checked …` and the refresh button are pushed to the right edge (a `grow` spacer, like `.tt` in the Board cards).

### 1.1 Items

| Item | Shown when | Look | Copy |
|---|---|---|---|
| **PR link** | `pr_number` set | `text.label`, `color.accent.default`, underline on hover | `PR #12`. Tooltip: the full URL. Click / Enter: `openUrl(pr_url)`. |
| **CI** | `pr.state == open` | filled 8px circle (`size.status-dot`) + `text.caption` mono count, no other icon | `● 4/6` (`done/total`). No jobs: grey `●` + `no checks`. |
| **Position badge** | `pr.state == open` | text, no fill; colored text per status | see §1.3 |
| **To push** | `pr.state == open` and `unpushed > 0` | quiet text | `↑2 to push` (1: `↑1 to push`). Tooltip: `2 commits on the integration branch not in the PR yet. The lead asks a push gate with request_pr.` |
| **Rebased** | `pr.state == open` and `rewritten` | attention-colored text | `rebased`. Tooltip: `The branch was rebased onto main. The next push replaces the PR's history (force-with-lease).` |
| **State badge** | `pr.state != open` | replaces CI, position, to push, rebased | `merged` (`status.done`) · `closed` (muted). |
| **Checked** | `pr` set | quiet, `text.caption` | `checked 2m ago` · `checked just now` (< 1 min) · `checked Tue 14:58` (> 24 h). Uses `ago()`: `2m`, `1h 05m`. Tooltip: the clock time (`checked 14:58`). |
| **Refresh** | `pr.state == open` (and while checking) | 16px icon button (󰑐 refresh glyph), ghost | Tooltip / label `Refresh PR status`. |

### 1.2 CI dot rule (user decision)

A **filled circle only**, one color from `pr.ci.overall`, then `done/total`:

| `ci.overall` | Dot | Means |
|---|---|---|
| `error` | red (`status.error`) | at least one finished job `failure`, `cancelled`, `timed_out`, `action_required`, `startup_failure`, `stale` or `error`. **Cancelled counts as an error.** Wins over every other color. |
| `running` | yellow (`status.queued`) | no error, at least one job `in_progress` |
| `waiting` | grey (`status.idle`) | no error, nothing running, at least one job not started (`queued`, `waiting`, `requested`, `pending`, `expected`) |
| `ok` | green (`status.done`) | every job finished, none in error (`success`, `neutral`, `skipped`) |
| `none` | grey (`status.idle`) | no job: the count reads `no checks` |

`done` counts every finished job whatever its outcome, so a red dot can sit next to `6/6` (all finished, one failed) or `3/6` (one failed early, others still running: red wins). The UI never recomputes `overall`: it takes the core's value.

### 1.3 Position badge

| `position.status` | Copy | Color | Tooltip |
|---|---|---|---|
| `up_to_date` | `up to date` | muted | `Contains every commit of main.` |
| `behind` | `↓3 behind main` | attention | `main has 3 commits the PR does not. The lead is told and rebases the feature onto main.` |
| `conflicts` | `conflicts` | error | `The PR conflicts with main. The lead is told and resolves them in a rebase.` |
| `unknown` | `—` | muted | `GitHub is still computing the PR's position against main.` |

`main` is `pr.base` (any base name, e.g. `↓3 behind develop`).

### 1.4 Every state of the line (with exact copy)

The states to draw. `PR #12` is always first; `checked …` + refresh always last unless said otherwise.

| # | State | Data | Line |
|---|---|---|---|
| 1 | **No PR** | `pr_url` null | unchanged: `No PR yet: the lead asks with request_pr after integration (PR gate).` (muted) |
| 1b | **Legacy PR** | `pr_url` set, `pr_number` null (a run done before this change, or a URL the core could not parse) | `PR #<n>` with the number parsed from `pr_url` (else the URL itself, truncated), as a link; nothing else (no CI, no refresh): the PR is not watched. |
| 2 | **Checking** (before the first check) | `pr_number` set, `pr` null | `PR #12` `checking…` (muted, with the refresh icon spinning, not clickable) |
| 3 | **CI waiting** | overall `waiting`, 0/6 | `PR #12` `● 0/6` (grey) `up to date` … `checked just now` `⟳` |
| 4 | **CI running** | overall `running`, 2/6 | `PR #12` `● 2/6` (yellow) `up to date` … |
| 5 | **CI green** | overall `ok`, 6/6 | `PR #12` `● 6/6` (green) `up to date` … |
| 6 | **CI red** (tooltip open) | overall `error`, 4/6 | `PR #12` `● 4/6` (red) `up to date` … + the job list (§1.5) |
| 7 | **No checks** | overall `none` | `PR #12` `● no checks` (grey) `up to date` … |
| 8 | **Behind** | `behind`, 3 | `PR #12` `● 6/6` `↓3 behind main` (orange) … |
| 9 | **Conflicts** | `conflicts` | `PR #12` `● 6/6` `conflicts` (red) … |
| 10 | **Position unknown** | `unknown` | `PR #12` `● 2/6` `—` … |
| 11 | **To push** | `unpushed` 2 | `PR #12` `● 6/6` `up to date` `↑2 to push` … |
| 12 | **Rebased** | `rewritten`, `unpushed` 4 | `PR #12` `● 6/6` `↓3 behind main` `↑4 to push` `rebased` … (behind stays until the push: the PR head on GitHub is still the old one) |
| 13 | **Stale** | `error` set | the line dims: every colored text (position, rebased) turns `color.text.muted` and the CI dot keeps its color but the count goes muted (no opacity: `opacity.disabled` 0.4 would break contrast); `PR #12` and the refresh button stay as they are; `checked` reads `last checked 12m ago` with a small error glyph before it (󰀦, `status.error`). Tooltip on it: `Last check failed: <error>. Showing the last known status.` |
| 14 | **Refreshing** | refresh call running | as the current state, refresh icon spins, `checked` reads `checking…` |
| 15 | **Merged** | `state == merged` | `PR #12` `merged` (green) … `checked 2m ago`. No refresh (the run is done, nothing is watched). |
| 16 | **Closed** | `state == closed` | `PR #12` `closed` (muted) … `checked 2m ago`. No refresh. If the lead opens a new PR, the line shows the new number from state 2 on. |

A paused / deferred run whose `resume_status` is `pr_open` shows the line as in `pr_open` (the last stored status; the watcher keeps running).

### 1.5 CI tooltip (the job list)

A popover anchored under the CI item, left-aligned to the dot. It holds links, so it is a real popover, not a native `title`.

```
┌──────────────────────────────────────────────────────────┐
│ CI · 4 of 6 finished · 1 failed · 1 running · 1 waiting  │  header: text.label; counts per state family
├──────────────────────────────────────────────────────────┤
│ ● ci / test                                       failed │  red dot
│ ● ci / e2e                                       running │  yellow dot
│ ● deploy-preview                                  queued │  grey dot
│ ● ci / lint                                       passed │  green dot
│ ● ci / build                                      passed │
│ ● ci / docs                                      skipped │  green dot, state word muted
├──────────────────────────────────────────────────────────┤
│ Open checks on GitHub ↗                                  │  footer: link to <pr_url>/checks
└──────────────────────────────────────────────────────────┘
```

- **Header**: `CI · <done> of <total> finished`, then the non-zero counts among `· N failed` (all error states), `· N running`, `· N waiting` (every not-started state). All green: `CI · 6 of 6 finished · all passed`. No checks: the popover shows `No CI checks on this PR.` and the footer only.
- **Row**: dot (same rule as §1.2 per job) · name · state word right-aligned (`text.caption`, the dot's color for error / running, muted otherwise). The name is `<workflow> / <name>` with the workflow in muted text; no workflow → the name alone.
- **Order**: the core's order (errors, running, waiting, the rest, each by name). The UI does not re-sort.
- **Link**: a row with `url` is a link (opens the job in the browser, hover = `color.surface.raised`); a row without `url` is plain text, no hover.
- **Size**: width 320px (min) to 400px (max); rows 24px. **Overflow**: past 12 rows the list scrolls (max height 12 rows = 288px) with header and footer fixed. A name too long for the row truncates with an ellipsis at the end; the state word never truncates; the full `workflow / name` is the row's tooltip.
- **Cap**: the core sends at most 100 jobs. When `total > jobs.length` the footer reads `+N more on GitHub ↗` instead of `Open checks on GitHub ↗`.

**Per-job state words** (GitHub `state` → word → dot):

| `job.state` | Word | Dot |
|---|---|---|
| `failure` | `failed` | red |
| `cancelled` | `cancelled` | red |
| `timed_out` | `timed out` | red |
| `action_required` | `action required` | red |
| `startup_failure` | `startup failure` | red |
| `stale` | `stale` | red |
| `error` | `error` | red |
| `in_progress` | `running` | yellow |
| `queued` | `queued` | grey |
| `waiting` | `waiting` | grey |
| `requested` | `requested` | grey |
| `pending` | `pending` | grey |
| `expected` | `expected` | grey |
| `success` | `passed` | green |
| `neutral` | `neutral` | green |
| `skipped` | `skipped` | green |
| anything else | the raw value, `_` → space | grey |

`skipped` and `neutral` are green because they do not block and the overall dot already counts them as ok; the word tells them apart from `passed`.

### 1.6 Behaviour at the card's min width (truncation order)

The right column is fixed at 440px, so the line has **406px** (card padding `space.lg` × 2 and the 1px border × 2). The worst case (`PR #123 ● 12/14 ↓12 behind main ↑12 to push rebased checked 59m ago ⟳`) does not fit. Items give way in this order, each step only when the previous is not enough:

1. `checked 2m ago` → `2m ago` (the full text moves to the refresh button's tooltip: `Refresh PR status · checked 14:58`).
2. `2m ago` hides.
3. `↓3 behind main` → `↓3 behind` (the base name stays in its tooltip).
4. `↑2 to push` → `↑2` (tooltip unchanged).
5. `rebased` → `↻` glyph with the same color and tooltip.

**Never shortened**: `PR #12`, the dot + `done/total` (or `no checks`), `conflicts`, `merged` / `closed`, the refresh button. In the stale state the error glyph stays even when `checked` hides (it moves next to the refresh button). Front can implement it with CSS (`flex-shrink` + container queries at the card width) or by measuring once per render; the order above is the contract.

### 1.7 Refresh

- Click / Enter / Space on the refresh button calls `pr_refresh { feature }`; the icon spins (or pulses its opacity with `prefers-reduced-motion`) until the call returns; the button is disabled meanwhile.
- Success: the line updates from the returned `run`, no toast.
- Failure: a toast `Couldn't check PR #12: <error>`; offline: `Offline: PR #12 not checked`. The line keeps its last status (and goes stale if the core set `error`).

## 2. Run state "PR open"

- Label **`PR open`** in `runStateLabel`, between `PR gate` and `Done` (sidebar meta, palette, Feature pane header, timeline `run.state` lines).
- Color **`color.status.working`** (cyan) in `runStateColor`.
  - Why: the run is live again: the tick, tasks and team tools all run, the team may be fixing CI. `at-gate` (magenta) is "PR gate", the user has to act; `done` (green) is now only "merged". `queued` (yellow) would read as CI running on the same screen, `waiting-for-you` (orange) as a user action, and `paused` (teal) as stopped.
  - It shares cyan with `Executing` / `Integrating`: the label tells them apart, and it's accurate: from the user's view the team is working until the PR merges. What the PR needs is on the PR line, not the run color.
- `runFinished` unchanged: the message box under the timeline stays while the PR is open.
- `Done` keeps green and now means merged. Timeline `run.state` lines read `PR gate → PR open (pull request opened)`, `PR open → Done (pull request merged)`, `PR open → Integrating (pull request closed)`.

## 3. Update gate card (Inbox)

Gate `action: "push_pr_update"`. `gateKind` gains `pr_update`; kind label **`PR update`** (the list's kind line and the detail header `PR update · lead`). It loads its detail like the PR gate (the payload carries what the card needs: no diff to fetch).

### 3.1 List card (left column)

- Title (from the core): `lead: push 2 commits to PR #12 (orch/improve-end-of-feature/integration)`; force: `lead: force-push PR #12 rebased onto main`.
- Preview line: the last commit's `sha subject` (mono, like other previews). Force: an error-colored `force-push` tag before the title, so it is visible in the list, not only in the detail.

### 3.2 Detail (right column), normal

```
PR update · lead                                             waiting 2m
Who   lead · orch/improve-end-of-feature/integration
What  push 2 commits to PR #12
Why   <detail from the core>

Branch   orch/improve-end-of-feature/integration → PR #12 ↗
Commits  2 new commits
         a1b2c3d  fix(T-48): retry flaky e2e login
         d4e5f6a  fix(T-49): lint
Approving pushes the branch with your credentials; agents never push. CI restarts on the PR.

[Approve: push 2 commits  a]   [Reject  r]
```

- `PR #12 ↗` is a link (`openUrl(url)`).
- Commits: `sha` 7 chars mono muted, then the subject (`text.label`, ellipsis at the end, full subject as tooltip). Up to **10**, then `+N more` (muted, not a link).
- Actions: `Approve: push 2 commits` (`a`, primary; 1 commit: `Approve: push 1 commit`) · `Reject` (`r`, danger), reject asks for a message like the PR gate.

### 3.3 Detail, force-push variant (`force: true`)

Same layout, with:

- **What**: `force-push PR #12 rebased onto main`.
- **Commits**: header `4 commits (rebased onto main)`: the list is the whole rewritten history the PR will show, not only new commits.
- A **warning block** right under the commits, before the note: error border (as `.nerr` in the Feature pane), error glyph 󰀦, text:
  > **Rewrites the PR's history:** rebased onto main. Pushed with force-with-lease: refused if someone else pushed to the PR since `8732db1`.
  
  (`8732db1` = `lease_sha`, 7 chars, mono.)
- Approve label: **`Approve: force-push`** (still primary: the warning carries the risk, the button names it).
- **Not in bulk approve**: a force-push gate is skipped when several marked items are approved with `a` (it must be selected and approved on its own); the multi-select count says `1 force-push skipped: approve it on its own`. Normal update gates can be bulk-approved like the PR gate.

### 3.4 After the decision

Same as other gates: the card leaves the list, a toast `Approved: pushed 2 commits to PR #12` / `Approved: force-pushed PR #12`; a refused lease shows as the gate's error toast `Push refused: someone pushed to PR #12 since 8732db1. The lead is told.` (the text comes from the core, the UI shows it as is).

## 4. Timeline copy

- `pr.opened`: `opened PR #12`
- `pr.status`: `PR #12 CI failed (ci / test, ci / e2e)` · `PR #12 CI ok` · `PR #12 behind main by 3` · `PR #12 conflicts with main` · `PR #12 up to date` (only the parts in `detail`; both changed: `PR #12 CI failed (ci / test) · behind main by 3`). More than 3 failed jobs: the first 3, then `+N`.
- `pr.pushed`: `pushed 2 commits to PR #12` · `force-pushed PR #12 (rebased)`
- `git.rebase`: `rebased the feature onto main (3 new commits from main)`; with `branches_moved`: `… · moved 2 branches`
- `pr.merged`: `PR #12 merged`
- `pr.closed`: `PR #12 closed without merge`

Families for the timeline filters: `pr.*` → **Git** (with `git.rebase`).

## 5. Keyboard and accessibility

- **Tab order** in the card: `PR #12` link → CI button → refresh button. The position / to push / rebased items are not focusable; their tooltip text is also their accessible description (`aria-describedby` or a visually hidden span), so a screen reader reads it with the line.
- **PR link**: `<a>` / button with accessible name `Pull request #12, opens in the browser`.
- **The dot is never the only carrier.** The dot is `aria-hidden`; the CI button's accessible name says the state in words:
  - `CI: 1 failed, 4 of 6 jobs finished. Show jobs` · `CI: running, 2 of 6 jobs finished. Show jobs` · `CI: waiting, 0 of 6 jobs finished. Show jobs` · `CI: passed, 6 of 6 jobs finished. Show jobs` · `CI: no checks`.
  - Each job row's name reads `test, workflow ci, failed` (the state word is visible text, so color-blind users get it in the popover too).
- **CI popover**: the CI item is a `button` with `aria-expanded` and `aria-controls`. Opens on hover (300 ms delay, stays open while the pointer is over the button or the popover, 150 ms grace to cross the gap) and on click / Enter / Space (pinned open). Focus moves to the first row only when opened from the keyboard. `↑` / `↓` move between rows, Enter opens the job, `Esc` closes and returns focus to the CI button, Tab past the footer closes it. Popover: `role="dialog"`, `aria-label="CI jobs for PR #12"`, the rows a list of links.
- **Refresh**: `aria-label="Refresh PR status"`, `aria-busy="true"` while running; the spin respects `prefers-reduced-motion`.
- **Live updates**: the line updates every minute; no live region (it would chatter). The events that matter (CI failed, merged, closed) reach the user through the timeline and the Inbox.
- **Stale**: dimming uses `color.text.muted` (AA in both themes), never opacity; the `last checked … · <error>` text is in the accessible description.
- **Gate card**: the force-push warning block is `role="note"` and is read before the actions; the approve button's accessible name is its visible label (`Approve: force-push`).
- **Contrast**: all colors are the `-aa` variants in light mode already; the 8px dot is decorative (the text carries the meaning), so it has no 3:1 requirement, but every status color clears 3:1 on `surface-pane` in both themes.

## 6. States to draw (UI checklist)

PR line (card at 440px): 2 checking · 3 grey `0/6` · 4 yellow `2/6` · 5 green `6/6` · 6 red `5/6` **with the tooltip open** (6 jobs) · the tooltip at overflow (20 jobs, scrolled) · 7 no checks · 8 behind · 9 conflicts · 10 unknown · 11 ↑ to push · 12 rebased (+ behind + to push) · 13 stale · 15 merged · 16 closed · the worst case truncated (§1.6).
Run state badge `PR open` (sidebar row, Feature pane header).
Update gate card: list card normal + force (with the `force-push` tag), detail normal, detail force-push.
