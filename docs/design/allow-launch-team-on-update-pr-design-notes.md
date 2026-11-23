# Allow to Launch a Team on Update PRs design notes: PR list, launch dialog, linked Feature pane, push gate, delete dialog

UX pass for allow-launch-team-on-update-pr, 2026-11-23, by `ux` for `ui`, then `front`. Sources: [spec](../../.orchestrator/features/allow-launch-team-on-update-pr/spec.md) (D1–D5), [API contract](../../.orchestrator/features/allow-launch-team-on-update-pr/api-contract.md), [UX brief](../../.orchestrator/features/allow-launch-team-on-update-pr/ux-brief.md). Read against the UI on main (`ProjectPane.svelte` Pull requests card, `Dialogs.svelte` launch / delete-feature, `FeaturePane.svelte` header and "PR and artifacts" card, `PrRows.svelte`, `PrLine.svelte`, `lib/inbox.js` PR gates, `lib/intake.js` `originChip`). Earlier notes stand: [P4.5](p4-5-design-notes.md) (prefilled dialogs §5, origin chip §6) and [Improve Intake](improve-intake-design-notes.md) (untrusted-text note, intake note slot).

Copy rules as before: sentence case, no exclamation marks, name the PR / feature / branch, every number says what it is. **Text that comes from GitHub (PR title, body, check names, log lines, author) is untrusted**: plain text only, never rendered as markdown outside the brief textarea. Branch names, slugs and paths are `mono`.

Section numbers follow ux-brief §2 (1–6); §7 is the keys and copy table, §8 the findings for lead, §9 the hand-off to ui.

**Principles applied (ux-brief §1)**
1. **Someone else's PR is named wherever the team touches it**: "PR #42 · `renovate/tokio-1.x` → `main` · renovate[bot]" (the **PR identity**) is the same string, in the same order, on the dialog, the Feature pane and the gate card.
2. **A new PR is never promised**: no surface says "Open a pull request" for a linked feature; the verbs are **Launch team…**, **Works on**, **Push**.
3. **Reuse**: every surface below is an existing one with a variant. **No new token.** The one warning (head moved) uses `status.queued` like the P4.5 over-room warning (no warning token exists; review as in P4.5).

## 0. Fixtures

One project, so the boards read as one story. Project `shop-api` (GitHub `acme/shop`, default branch `main`), team "Default" v3.

| PR | Title | Head → base | Author | CI | Review | Draft | Launch state |
|---|---|---|---|---|---|---|---|
| #42 | chore(deps): update rust crate tokio to v1.48 | `renovate/tokio-1.x` → `main` | renovate[bot] (bot) | 5 passing · 1 failing | none | no | launchable |
| #43 | chore(deps): update dependency vite to v7 | `renovate/vite-7.x` → `main` | renovate[bot] (bot) | 3 passing · 2 pending | none | no | active team: `pr-43-update-dependency-vite`, executing |
| #44 | Bump serde to 1.0.230 | `dependabot/cargo/serde-1.0.230` → `main` | dependabot[bot] (bot) | 6 passing | approved | no | earlier team: `pr-44-bump-serde`, done |
| #45 | Add retry to the mailer | `mia-dev:feat/mailer-retry` → `main` | mia-dev (fork) | 4 passing · 1 failing | changes requested | no | fork, disabled |
| #46 | wip: split the cart module | `lena/cart-split` → `main` | lena | none reported | none | **yes** | launchable (draft) |

Linked feature for the Feature pane boards: `pr-42-update-rust-crate-tokio`, run phases used: executing (before push), pr gate, pr open, head moved, closed. Branch `renovate/tokio-1.x`, head at link `9f2c1ab`, someone else's push `a41d7e0`, team commits 3.

---

## 1. PR list row (Project › Pull requests)

### 1.1 Anatomy

Today a row is one `<p>`: `#42 title (draft) head → base · CI … · review`. It becomes a **two-line row** with the action on the right, in the same card (`Pull requests`, head: title, `gh` label, **Load PRs**). Rows are separated by hairlines, 8 px vertical padding, no box.

```
#42  chore(deps): update rust crate tokio to v1.48        [draft]         [ Launch team… ]
     renovate/tokio-1.x → main · 󰚩 renovate[bot] · CI 1 failing · no review
```

- **Line 1**: `#42` (bold, mono, link-coloured only on hover) · title (`text.primary`, ellipsis, cut first) · **draft** chip (same chip as the current "(draft)", `text.muted` outline) · action, right-aligned, never shrinks. The number and the title are one **link button** that opens the PR on GitHub (`openUrl(url)`); tooltip "Open PR #42 on GitHub".
- **Line 2** (`caption`, `text.muted`): `head → base` (mono) · author · CI · review decision. Joined by ` · `, each piece present only when it has a value. Same order as today with the author inserted after the branches.
  - **Author**: the login; for `author.is_bot` a robot glyph (U+F06A9, ui confirms it exists in the shipped Nerd Font) before it, `aria-label="bot"`. A human author has no glyph. A null author (deleted account) shows `unknown author`.
  - **CI**: the text the pane builds today from `checks` (`ci(pr.checks)`); failing parts in `status.error`, nothing else coloured.
  - **Review**: `changes requested` / `approved` as today (lower-cased `review_decision`); absent when null.
- Line 2 wraps under the title at narrow widths; the action stays on line 1. The label always stays "Launch team…": the button is `flex: none` and the title shrinks.

### 1.2 The action, per state (D3, §5.1)

The state comes from the row's `launch` and `feature` (api-contract §1). One button per row, `btn` (secondary, not primary: the pane's primary is Load PRs).

| State | Condition | Line 1 action | Line 2 extra | Notes |
|---|---|---|---|---|
| **Launchable** | `launch.ok` and (`feature` null or its `run_status` is `done` / `failed`) | **Launch team…** (enabled) | none | Opens the PR dialog (§2). |
| **Launchable, earlier team** | as above and `feature` set | **Launch team…** | `Earlier team: pr-44-bump-serde · done` where the slug is a link button → opens that feature | Failed: `· failed`. A finished feature does not hide the action (D3). |
| **Active team** | `launch.code == "active_feature"` | **Open feature** (enabled) | `Team on this PR: pr-43-update-dependency-vite · executing` (slug is not a link: the button is the link) | `run_status` is shown with the same phase word the Feature pane header uses (spec, spec gate, planning, executing, pr gate, pr open…). |
| **Fork** | `launch.code == "fork"` | **Launch team…** disabled (`aria-disabled`, still focusable) | `Fork PR: the app pushes to origin only` (visible text, `text.muted`, linked to the button with `aria-describedby`) | Tooltip on the button repeats the reason. The reason is the core's `launch.reason`; the copy above is the default. Enter / click do nothing. |
| **Draft** | `is_draft` | as Launchable | none | Drafts are allowed (D4). The **draft** chip is the only mark. |
| **No `launch` field** | an older core | no action | none | Defensive: nothing is offered rather than a button that cannot be explained. |

Closed and merged PRs are not listed (the list is open PRs only), so a PR that closes while the list is on screen stays until the next Load PRs; the dialog's prefill (§2.6) is where that is caught.

**After a launch from a row** the dialog closes as every launch does; the cached `prs` are **not** reloaded by the pane's topic refresh (they come from `withPrs: true`), so front patches that row locally (state **Active team**, `feature` from the launch result) and the next Load PRs confirms it.

### 1.3 Key

The pane has `f` fetch, `p` load PRs, `t` trust, `x` remove. Rows become focusable (Tab enters the list, **↑ / ↓** move between rows, the focused row has the usual focus ring, no row is focused by default).

| Key (row focused) | Does |
|---|---|
| `Enter` | The row's action (Launch team… / Open feature); nothing on a disabled one |
| `l` | Launch team… (same as Enter on a launchable row; a no-op elsewhere) |
| `o` | Open the PR on GitHub |

No conflict with the pane's keys; ui and front check against the Project pane map and the leader keys on the 05 board before shipping (`o` is also a leader step, but leader keys need the ⌘; prefix).

### 1.4 States of the card

| State | Shows |
|---|---|
| Not loaded | `Not loaded (a network call).` (as today) |
| Loading | **Load PRs** reads `Loading…` and is disabled; previous rows (if any) stay on screen at 60 % opacity, actions disabled; first load: `Loading pull requests from gh…` in `text.muted` |
| Empty | `No open pull request.` (as today) |
| `gh` error | the core's text (`prs.error`) in `text.muted` as today (gh missing, not signed in, offline); **Load PRs** retries. No extra copy |
| Loaded | rows as §1.1–1.2 |

---

## 2. Launch dialog, PR variant

The existing **Launch {team}** dialog, prefilled size (600 wide at both widths, board "P4.5 · Launch dialog · prefilled"), opened by **Launch team…**. Title keeps "Launch Default" (team name from the Team select).

### 2.1 Layout (top to bottom)

```
Launch Default
From PR #42 · GitHub ↗                                     ← origin chip
Project  [shop-api (locked)]        Team  [Default ▾]
Slug     [pr-42-update-rust-crate-tokio]    Works on  PR #42 ↗ · renovate/tokio-1.x → main · 󰚩 renovate[bot]
Title    [PR #42 · chore(deps): update rust crate tokio to v1.48]
Brief (the PR and its failing checks, as markdown)
 ┌ textarea, 12 rows ─────────────────────────────────────────┐
 │ Make PR #42 pass CI: chore(deps): update rust crate …      │
 │ …                                                          │
 └────────────────────────────────────────────────────────────┘
Issues (optional)  [ chips ]
                                                  (no Pull requests control)
 • Checks out renovate/tokio-1.x from origin in the lead's worktree. Nothing is branched from main.
 • The lead drafts spec.md … in the feature's docs folder, then asks for the Spec gate.
 • The team's commits are pushed to PR #42 only when you approve the push. No new pull request is opened.
 • Specialists start after the gate, as the Governor and the live-agent cap allow.
☐ Run off-hours
 This text comes from GitHub (the pull request and its check logs). It goes to the agent as part of the brief you send.
[Cancel]                                                   [Launch]
```

### 2.2 What changes against the prefilled intake dialog

| Part | Intake dialog | PR variant |
|---|---|---|
| **Origin chip** (`originChip`) | `From #13 Address step · in Checkout v2 · GitHub ↗` | `From PR #42 · GitHub ↗`. Built from `origin {kind:"pr", label:"PR #42", url}`; the PR title is the Title field's, so it is not repeated. The ↗ opens the PR (`aria-label="Open PR #42 in GitHub"`); the tooltip is the PR title. Same chip style and place as the other origins. |
| **Feature** segment (New / Existing) | shown | **Hidden**: a PR launch always creates a new feature (see F1). |
| **Project** | select | read-only (the `shop-api` name as static text in the field style, no chevron): the PR belongs to it. |
| **Slug** | editable | editable, prefilled `pr-42-update-rust-crate-tokio` (≤ 40, unique, from the core). |
| **Base** | editable input | **Replaced by Works on** (same cell, same width). |
| **Pull requests** control | per-issue / single | **Hidden** (`pr_mode` is single; spec §3). The Issues field stays (task tracking only). |
| **What it does** bullet 1 | `Creates orch/<slug>/integration and the lead's worktree on it.` | `Checks out renovate/tokio-1.x from origin in the lead's worktree. Nothing is branched from main.` (real branch, mono). |
| **What it does** new bullet | none | `The team's commits are pushed to PR #42 only when you approve the push. No new pull request is opened.` (after the docs bullet). |
| **Untrusted note** (`data-untrusted`) | `This text comes from GitHub. …` | `This text comes from GitHub (the pull request and its check logs). It goes to the agent as part of the brief you send.` |
| **Intake notes** (ran before, blocked, write-back, estimate) | per item | none (the PR is not a tracker item; no estimate line in v1). |

### 2.3 Works on (read-only)

A static line in the field's cell, label `Works on` (same label style as Base, no input border, so it is not mistaken for something editable): `PR #42 ↗` (link button, opens the PR) · `renovate/tokio-1.x → main` (mono) · author with the bot glyph. A draft adds `· draft`. The whole line has the tooltip "The pull request and its branches. Fixed: the team works on this branch." It wraps to two lines at 600 wide when the branch is long (author goes to line 2); branch names are never cut, a very long one (> 48 characters) is cut in the middle with the full name in the tooltip.

### 2.4 Brief and the failing-check excerpts

- Label: `Brief` + muted `(the PR and its failing checks, as markdown)`; textarea **12 rows** (as every prefilled dialog), editable, **scrolls inside**; the dialog does not grow.
- **How long it may look before scrolling**: nothing more than 12 rows. The template (api-contract §2) puts the PR line, the work instruction and the first failing check's name and the start of its log in those 12 rows; the rest (up to 4 jobs × 60 lines, 12 KB) is reached by scrolling. The excerpt is the **tail** of the log, so a job's error is at its end: front scrolls the textarea to the **top** on load (the PR line first), not to the end.
- Under the textarea, only when the core says logs were cut (see F2): `Logs are cut to the last 60 lines of each failing job, 4 jobs at most.` in `text.muted`, `caption`. When there is no failing check the template's own line (`No check is failing right now (1 pending).`) says so; nothing else is added.

### 2.5 Prefill loading state

`pr_prefill` takes seconds (`gh pr view`, then a `gh run view --log-failed` per failing job). The dialog **opens at once** in its PR layout, in a loading state:

- Origin chip: `From PR #42 · GitHub ↗` is already shown (the row has the number and url).
- Project, Team, Slug, Title, Brief, Issues, Run off-hours: **disabled**; Slug / Title show a skeleton bar, the Brief textarea shows a muted `Reading PR #42, its checks and the failed logs from GitHub…` placeholder; **Works on** is already filled from the row (`head_ref`, `base_ref`, `author`) since the list has them.
- **Launch** is disabled; **Cancel** and Esc close the dialog and the late response is dropped.
- After 8 s the placeholder gains a second line: `Still reading. Failing logs can take a while.`
- On success the fields fill, **Launch** takes the focus (as the other prefilled dialogs: Enter launches, Tab goes to Title).

### 2.6 Prefill errors (dialog body replaced)

The form is replaced by one `err` block (`role="alert"`) under the chip; the footer is **[Close]** (and **[Try again]** where marked). The PR list is reloaded in the background after `pr_closed`, `pr_merged`, `active_feature` so the stale row disappears.

| Code | Copy | Buttons |
|---|---|---|
| `pr_closed` | `PR #42 was closed on GitHub. A team can't work on it.` | Close |
| `pr_merged` | `PR #42 was merged on GitHub. There is nothing left to fix.` | Close |
| `fork` | `PR #42 comes from a fork (mia-dev/shop). The app pushes to origin only, so it can't launch a team on it.` (the core's message when present) | Close |
| `active_feature` | `PR #42 already has a team: pr-43-update-dependency-vite (executing).` | **Open feature**, Close |
| `gh_missing` | `The GitHub CLI (gh) isn't installed. Install it, run gh auth login, then try again.` | Try again, Close |
| `gh_auth` | `gh isn't signed in to GitHub. Run gh auth login in a terminal, then try again.` | Try again, Close |
| `offline` | `Couldn't reach GitHub. Check your connection and try again.` | Try again, Close |
| other / unknown | `Couldn't read PR #42: {message}` | Try again, Close |

**Try again** re-runs `pr_prefill` and returns to the loading state. `gh auth login` is plain text, not a button (the app does not run it).

### 2.7 Launch errors (inline, form kept)

Shown in the existing `err` slot above the footer; the form stays editable and **Launch** stays enabled for the four fixable ones (the user fixes it outside the app, or edits and retries). Paths are shortened with `~` as the dialogs do (`shortPath`).

| Code | Copy | After |
|---|---|---|
| `branch_checked_out` | `renovate/tokio-1.x is checked out in ~/dev/shop. Switch that checkout to another branch, then launch again.` | Launch enabled |
| `local_ahead` | `Your local renovate/tokio-1.x has 2 commits that PR #42 does not have. Push or delete them first, then launch again.` | Launch enabled |
| `fetch_failed` | `Couldn't fetch renovate/tokio-1.x from origin: {message}. Check the connection and try again.` | Launch enabled |
| `pr_closed` | `PR #42 was closed since this dialog opened. A team can't work on it.` | Launch disabled, Cancel only |
| `fork` / `active_feature` | as §2.6 (the same strings, in the slot) | Launch disabled; `active_feature` adds **Open feature** next to Cancel |

Nothing is created when a launch fails (spec §5.3 checks before creating), and the copy never says a worktree was left behind. While launching the button reads `Launching…` as today.

---

## 3. Feature pane of a linked feature

The PR identity appears in two places, both from launch (before any push).

### 3.1 Header row 2

`metaText` becomes `Default v3 · \u{E725} renovate/tokio-1.x · PR #42 · started 12 min ago` (pieces joined by ` · `): the branch piece uses `feature.branch` (api-contract §4: always present; an unlinked feature shows `orch/<slug>/integration`, so the hard-coded `FeaturePane.svelte:73` text goes), and a **`PR #42`** piece is added after it for a linked feature (link button, opens the PR; `aria-label="Open PR #42 on GitHub"`). The Docs card head `on the integration branch` stays correct (the integration branch is the PR branch).

### 3.2 "PR and artifacts" card

For a linked feature the card gets a **linked line** above the PR line, and the "No PR yet" paragraph never appears (the run has `pr_url` from launch).

```
PR and artifacts                                                        (card head, unchanged)
Works on PR #42 ↗ · renovate/tokio-1.x → main · opened by 󰚩 renovate[bot]      ← linked line (caption)
● PR #42 · CI 1 failing · behind main …                        checked 2 min ago ⟳ ← PrLine (existing)
Nothing pushed to PR #42 yet. The lead asks you to approve the push when the work is done (Push gate).
```

- **Linked line**: `Works on` (muted) + `PR #42 ↗` (link) + `head → base` (mono) + `opened by {author}`; shown in every state below, so the card always says whose PR it is.
- **PrLine**: the existing component. From launch the watcher fills `run.pr`, so it shows the PR's real CI and state (`checking` for the first seconds, via the existing `checking` state). It is the same line before and after the push.
- **Pre-push line** (until `pr_open`; `text.muted`): `Nothing pushed to PR #42 yet. The lead asks you to approve the push when the work is done (Push gate).` With a pending gate: `The lead asked to push to PR #42.` + the existing **Show card** link button. After the push the line is gone.
- **Per-issue rows** (`PrRows`) are not used on a linked feature (no PR mode).

### 3.3 Head moved outside the app (`run.pr.foreign_head`)

A warning block (`warnline`: glyph U+F0026, `status.queued`, as the dialog's blocked line) between the linked line and the PrLine:

`PR #42's head moved: a41d7e0 was pushed outside the app, probably by renovate[bot]. The lead was told and brings it in with a rebase before asking for the push.` (`a41d7e0` is a link button to the commit on GitHub when the sha is known, see F4; the "probably by" clause is dropped for a human author: `was pushed outside the app.`)

It has no button: the lead acts. It disappears when the branch contains the new head (the core resets `foreign_head`). The same block shows in `pr_open`.

### 3.4 Closed and merged

- **Closed** (PrLine's existing `closed` state; run `integrating`): the pre-push line is replaced by a `status.error` line: `PR #42 was closed on GitHub. The team can't push to it, and the app never opens another pull request. Reopen it on GitHub, or delete this feature.` The Push gate cannot be raised (`request_pr` is refused), so there is no Show card. See F5 for what happens if it is reopened.
- **Merged**: PrLine's existing `merged` state; run `done`; the linked line stays; no extra copy.

### 3.5 Other surfaces (no new design, named for front)

- **Spec gate card**: gains one muted line under its title for a linked feature: `Works on PR #42 (renovate/tokio-1.x), opened by renovate[bot]. Approving lets the team change that branch.` (D5: the gate is where the user sees what the team will change on someone else's PR). Built from `feature.pr_link`; no contract change.
- **Mission control / sidebar rows**: unchanged in v1 (they show the slug and phase). Not designed.

---

## 4. Gate card: push to the PR

`push_pr` with `linked` (api-contract §6). Same card component and place as the other PR gates (`prGate()` in `lib/inbox.js` adds `title`, `sub`, `note`).

```
Push 3 commits to PR #42                      (card title; 1 commit: "Push 1 commit to PR #42")
renovate/tokio-1.x → main · opened by 󰚩 renovate[bot]                         (sub)
Goes to the existing branch of someone else's PR. No new pull request is opened.   (note)
┌ diff block: the team's changes on top of the PR (origin/renovate/tokio-1.x…HEAD) ┐
│ 3 commits · 5 files · +42 −7        (file list as the other PR gates' diff block)│
└──────────────────────────────────────────────────────────────────────────────────┘
Pushes with a lease on 9f2c1ab, the head the app last saw. If someone pushed since, nothing is overwritten.
                                                      [ Decline ]   [ Push ]
```

- **Title**: `Push {n} commit(s) to PR #{number}` (singular at 1). Not `Open a pull request`. **Sub**: PR identity without the PR number (the title has it): `head → base · opened by {author}` (author from `feature.pr_link`; omitted if unknown).
- **Note**: one line, `text.muted`, exactly: `Goes to the existing branch of someone else's PR. No new pull request is opened.`
- **What it lists**: only the team's commits and files (`diff` is over `origin/<head>...HEAD`); the block's header reads `Your team's changes on top of PR #42`. The "Closes #12" line (single-mode intake) is **not** shown on a linked gate: the PR is not ours to close issues on.
- **Lease line** (`caption`, `text.muted`): `Pushes with a lease on {lease7}, the head the app last saw. If someone pushed since, nothing is overwritten.`
- **Buttons**: approve is **Push** (primary), decline keeps the other PR gates' label and keys (the card's existing key map). No new key.
- **Head moved while the card is open** (`run.pr.foreign_head`): a `warnline` under the note: `PR #42's head moved since this request. The push will be refused until the lead rebases on it.` The buttons stay enabled (the core's refusal is the safety; the user may decline).
- **Outcomes** (the card collapses to one line, as every resolved card):
  - approved and pushed: line `Pushed 3 commits to PR #42`, toast `Pushed to PR #42` (api-contract §6).
  - **refused, head moved** (the lease check failed): the card closes as failed with `Not pushed: PR #42 moved since the request. The lead was told and rebases on it, then asks again.` (`status.error`, line in the card and in the chat's collapsed line). See F6.
  - refused, PR closed: `Not pushed: PR #42 was closed on GitHub.`
  - push error (network, auth): the existing PR gate failure copy with the core's message.

---

## 5. Delete feature dialog (linked feature)

Same `Delete feature {slug}?` dialog (small, `alertdialog`). For a linked feature (`feature.pr_link` set):

- Bullets (existing three) plus a fourth: `• PR #42 stays open on GitHub. Nothing is deleted there.`
- Checkbox 1 (replaces `Remove its worktrees and branches, orch/<slug>/integration included`):
  `Remove its worktrees and local branches. renovate/tokio-1.x is removed only if it is already pushed to PR #42; otherwise it is kept.`
- Checkbox 2 (shown when 1 is checked): `Discard unmerged work (otherwise unmerged worktrees are kept)`. For a linked feature it does **not** delete the PR branch's unpushed commits (D2), so the label reads `Discard unmerged work in the worktrees (commits on renovate/tokio-1.x that PR #42 does not have stay on the local branch)`.
- **Kept line** (only when the core can tell, see F7), under checkbox 1 when it is checked, `text.muted`: `renovate/tokio-1.x has 2 commits that PR #42 does not have: it will be kept.` With no unpushed commit: `renovate/tokio-1.x is pushed: it will be removed locally.`
- **Button** unchanged: `Delete feature`.
- **Result toast**: `Deleted pr-42-update-rust-crate-tokio` + when a linked branch was kept ` · kept renovate/tokio-1.x: 2 commits not pushed to PR #42` (spec §5.7's text). A kept branch is not an error: no `error` tone.
- An unlinked feature's dialog is unchanged (real `branch` replaces the hard-coded name, which is the same string).

---

## 6. States matrix

| # | Surface | State | What shows |
|---|---|---|---|
| 1 | PR list | launchable, human author | row §1.1, **Launch team…** |
| 2 | PR list | launchable, bot author | + robot glyph |
| 3 | PR list | draft | + **draft** chip, **Launch team…** |
| 4 | PR list | earlier team done / failed | **Launch team…** + `Earlier team: … · done` |
| 5 | PR list | active team | **Open feature** + `Team on this PR: … · executing` |
| 6 | PR list | fork | disabled **Launch team…** + `Fork PR: the app pushes to origin only` |
| 7 | PR list | loading (first / reload), empty, gh error | §1.4 |
| 8 | Dialog | loading | §2.5 (skeleton, disabled, Works on filled) |
| 9 | Dialog | prefilled | §2.1 (Launch focused) |
| 10 | Dialog | prefilled, logs cut | + the cut line §2.4 |
| 11 | Dialog | prefill error: closed / merged / fork / active / gh / offline | §2.6 (one board with the six variants) |
| 12 | Dialog | launch error: checked out elsewhere / local ahead / fetch failed / closed | §2.7 |
| 13 | Feature pane | linked, before push (executing) | header PR piece, linked line, PrLine, pre-push line |
| 14 | Feature pane | linked, push gate pending | pre-push line `The lead asked to push to PR #42.` + Show card |
| 15 | Feature pane | linked, after push (pr open) | linked line + PrLine, no pre-push line |
| 16 | Feature pane | head moved | warning §3.3 |
| 17 | Feature pane | closed | §3.4 |
| 18 | Feature pane | merged | PrLine merged, run done |
| 19 | Gate card | push, 3 commits / 1 commit | §4 |
| 20 | Gate card | push, head moved while open | warning line |
| 21 | Gate card | pushed / refused (moved) / refused (closed) | collapsed outcomes §4 |
| 22 | Delete dialog | linked, nothing checked / checked with kept line / no unpushed | §5 |

Dark is the reference; light follows the same tokens (no new one).

---

## 7. Keys and copy table (for the 05 board and the palette)

| Where | Key | Does |
|---|---|---|
| Project pane, PR row focused | `↑` `↓` | move between rows |
| | `Enter` / `l` | Launch team… / Open feature |
| | `o` | Open the PR on GitHub |
| PR dialog | `Esc` | Cancel (drops a late prefill) |
| | `Enter` (Launch focused) / `⌘Enter` | Launch |
| Push gate card | existing PR gate keys | Push / Decline |

Palette: no new command. The existing launch entries do not take a PR.

Strings (exact), in addition to §1–§5: `Launch team…` · `Open feature` · `Fork PR: the app pushes to origin only` · `Earlier team: {slug} · {run status}` · `Team on this PR: {slug} · {run status}` · `From PR #42 · GitHub` · `Works on` · `Checks out {branch} from origin in the lead's worktree. Nothing is branched from main.` · `Push {n} commit(s) to PR #{n}` · `Pushed to PR #42` · `Nothing pushed to PR #42 yet. …`

---

## 8. Findings for lead

| # | Finding | Why it matters | Proposal |
|---|---|---|---|
| **F1** | D3 says a finished feature on the same PR "can be relaunched (existing-feature path)", but the dialog's **Existing: slug** segment launches a team on a feature with no run, with its own brief/slug, and a PR launch carries `pr` for a **new** feature. Two features on one PR also collide on the branch: the finished one's worktree still has `renovate/tokio-1.x` checked out, so the second launch fails with `branch_checked_out`. | The row for #44 ("earlier team, done") offers Launch team…; the user will hit that error without knowing why. | v1: the PR dialog has **no** Feature segment (always a new feature); `branch_checked_out`'s message must name the earlier feature's worktree when that is the cause (`… checked out in ~/dev/shop.worktrees/pr-44-bump-serde/integration: delete that feature first`). Confirm, or say what "existing path" means. |
| **F2** | `pr_prefill` returns only the brief text. The dialog needs to know when logs were cut or missing (§2.4) and how many checks fail. | Without it the dialog cannot tell "no failing check" from "logs not available" and cannot show the cut line. | Add `prefill.notes: { failing: 2, logs: "full" \| "cut" \| "annotations" \| "none" }` (optional; the cut line shows only for `cut`). |
| **F3** | `branch_checked_out` and `local_ahead` are launch-time errors, so the user fills in and submits a dialog that was always going to fail. | A pre-flight check in the prefill is free (the fetch is not needed for either). | Add `prefill.warnings: [{code, message}]` for `branch_checked_out` and `local_ahead`; the dialog shows them as §2.7 from the start (Launch still enabled: the user may fix outside and press it). Optional; §2.7 works without. |
| **F4** | `run.pr.foreign_head` is a bool. The warning (§3.3) names the new head and the gate card names the stale lease. | "The head moved" without a sha or author cannot be checked on GitHub. | Add `run.pr.head_sha` (the last seen head) and `run.pr.foreign_head_sha` (the moved one) and the author login of its commit if cheap (else the copy keeps "probably by {pr author}"). |
| **F5** | A closed linked PR leaves the run `integrating` and `request_pr` refused. If the user **reopens** it on GitHub, nothing says the watcher resumes. | §3.4 tells the user to reopen it or delete the feature. | Confirm that `pr_tick` clears the closed state when the PR is open again (and the `request_pr` refusal with it). If not, the copy drops "Reopen it on GitHub". |
| **F6** | A push gate approved with a moved head is refused ("PR #42 moved since the request: rebase …"), but the contract does not say what the **card** becomes. | The card must end in a clear state; a gate left pending would be asked twice. | The gate resolves as **failed** with a machine code (`pr_moved`, `pr_closed`, `push_failed`) and the message; the lead gets the ORCH turn. UI copy §4 assumes this. |
| **F7** | The delete dialog has no plan for a feature: it cannot say how many commits are not on the PR (§5). Agents have `delete_plan`, features do not. | The checkbox copy is accurate but generic; the user cannot see what will be kept. | Add to the feature (or a `feature_delete_plan`): `branch_unpushed: <commits>` for a linked feature; and have `feature_delete` return `kept: [{branch, commits}]` for the toast. The generic copy ships without them. |
| **F8** | The row's cached `prs` are not refreshed by the `project:` topic. After a launch from a row the row is stale until the next Load PRs. | The row would still say **Launch team…** and invite a second launch (refused with `active_feature`). | Front patches the row locally from the launch result (§1.2). Or the launch result returns the row's `feature` object; please include `{id, slug, run_status}` in it. |
| **F9** | `author` can be null (deleted GitHub account) and `author.is_bot` can be false for a `[bot]` login. | Row and dialog must not crash or lie. | Contract says `is_bot` is derived from the login suffix too: keep it; state in the contract that `author` may be null (UI shows `unknown author`). |
| **F10** | `active_feature` errors (prefill and launch) must say **which** feature to open. | §2.6's **Open feature** button needs `id` and `slug`. | `{code, message, feature: {id, slug, run_status}}` on `active_feature`. |
| **F11** | Launch errors carry the path / count only inside `message`. | The UI wants the path in mono and `~`-shortened, and the commit count in plain words. | Optional: `details: {path}` for `branch_checked_out`, `{commits}` for `local_ahead`; the UI falls back to the message. |

---

## 9. Hand-off to ui (FigCraft boards, ux-brief §3)

Start from the Project pane page (Pull requests card), page **04.4** (Launch dialog, "P4.5 · Launch dialog · prefilled", Origin chip), the PR gate card components on 04.3, the PrLine and PR card boards, and the delete-feature dialog. No new token; the bot glyph is the one new mark (check it in the shipped Nerd Font).

| Board (1512 and 1280 where the layout changes) | Content | Notes § |
|---|---|---|
| `pr-list` | Project pane Pull requests card with the five fixtures (#42 launchable bot, #43 active team, #44 earlier team done, #45 fork disabled, #46 draft), plus loading / empty / gh error; focused row and keys | 1, 6 rows 1–7 |
| `pr-launch-dialog` | prefilled (#42), loading, logs-cut line, the six prefill errors, the four launch errors; 600 wide at both widths | 2, 6 rows 8–12 |
| `pr-feature-linked` | Feature pane header + PR card: before push, push gate pending, after push (pr open), head moved, closed, merged; spec gate line | 3, 6 rows 13–18 |
| `pr-gate-push` | push gate card (3 commits, 1 commit), head moved while open, outcomes (pushed, refused moved, refused closed) | 4, 6 rows 19–21 |
| `pr-delete-feature` | linked delete dialog: default, checked with kept line, no unpushed | 5, 6 row 22 |

Export each as `pr-<board>-1512.png` / `-1280.png` under `.orchestrator/features/allow-launch-team-on-update-pr/design/`, add the rows to `docs/design/README.md`, commit exports and notes. Boards drawn from the fixtures in §0 so they read as one story. ux does not change code and does not edit the FigCraft file.
