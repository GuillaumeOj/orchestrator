# X2 · UX check of the built PR launch flow

By `ux`, 2026-11-23, on the integration tip `ec09cf2f`. Read against the design notes
(`docs/design/allow-launch-team-on-update-pr-design-notes.md`) as amended by api-contract §8–§9, and against the
built UI and its tests (`prlaunch.test.js`, `prlink.test.js`, `feature-linked.test.js`, `decisions.test.js`). Read only:
no code changed, the app was not run (copy, keys and states are checked in source and tests).

**Result: no must fix.** The built copy matches the notes and §8–§9 on every surface. Eleven acceptable deviations
and four test gaps below; none blocks the PR.

## Must fix

None.

## Acceptable

### PR rows (`ui/src/components/PrListRow.svelte`, `ui/src/lib/prlaunch.js`)
| # | Where | What differs from the notes | Why it is acceptable |
|---|---|---|---|
| A1 | `PrListRow.svelte:50` | A row whose `launch` is refused with a code other than `fork` / `active_feature` (e.g. the unusable-branch refusal B2 added) shows no button and the core's `launch.reason` in line 2. Notes §1.2 only knew "no `launch` field → nothing". | Same rule (nothing offered that cannot be explained), and the reason is visible. |
| A2 | `PrListRow.svelte:14`, `ProjectPane.svelte:~325` | `onfeature(pr.feature)` runs `openFeature(f.id)`: a row with `launch.code == "active_feature"` and a null `feature` would throw on click. | The contract (§1) gives `feature` whenever the code is `active_feature`; no known path breaks it. A `?.` guard would be cheap. |
| A3 | `PrListRow.svelte:34` | The row is a `li tabindex=0`; Enter acts only when the row itself is focused, `l` and `o` act from the inner buttons too. | Exactly §1.3. |
| A4 | `prlaunch.js:139` | Fork reason: "Fork PR (from mia-dev): the app pushes to origin only." | The §9 copy (no repo name). The core's `launch.reason` is not used for forks; the UI builds the string from `fork_owner`. |

### Launch dialog (`ui/src/components/Dialogs.svelte`)
| # | Where | What differs | Why acceptable |
|---|---|---|---|
| A5 | `Dialogs.svelte:~613` | Bullet 1 reads "Nothing is branched from {base_ref}" (notes: "from main"). | More exact for a PR against another base. |
| A6 | `Dialogs.svelte:~614` | Bullet 2 drops "in the feature's docs folder" and ends "The feature docs stay in the app; only the team's commits go to the PR." | The §8 F12 copy. |
| A7 | `prlaunch.js:189` | Fork prefill error has no "(mia-dev/shop)" repo name. | §9 (no repo name; no data). The core's message wins when present. |
| A8 | `Dialogs.svelte:~564` | Prefill errors: Close is the primary button (and focused) when there is no other action; with Open feature / Try again, that one is primary and focused. | Front's ui-review fix; the notes left it open. |
| A9 | `Dialogs.svelte:~590` | Loading placeholder's second line ("Still reading. Failing logs can take a while.") is a `\n` in the textarea `placeholder`. | WebKit renders newlines in a textarea placeholder; if a webview drops it the first line still reads right. Worth a glance when someone runs the app. |
| A10 | `prlaunch.js:228` | `branch_checked_out` copy is built from `details.path` / `details.feature` ("… checked out in ~/…/integration (feature pr-44-bump-serde): delete that feature first, then launch again."), the core's message when there is no path. | Matches api-contract §8 F1 and F11. |

### Feature pane, gate, delete
| # | Where | What differs | Why acceptable |
|---|---|---|---|
| A11 | `prlink.js:360`, `DecisionCard.svelte` | The decline button keeps the core's label **Reject** (the notes said "keeps the other PR gates' label"; they say Reject). The core's own title keeps a branch suffix ("Push 3 commits to PR #42 (renovate/tokio-1.x)", `decision.rs` linked gate) while the card shows the notes' title without it; the branch is in the sub line. | The card is as designed; the suffix only shows where the core title is used (lists, notifications). |
| A12 | `prlink.js:383` | A `push_failed` refusal prints "Not pushed: PR #42 could not be read (…): nothing was pushed." (the core's message already says "nothing was pushed"). | Reads fine, slightly redundant. |
| A13 | `FeaturePane.svelte:~470` | The "No PR yet: the lead asks with request_pr…" paragraph can still show on a linked feature with no run / no `pr_url`. | Notes §3.2 say it never appears once the run exists (it has `pr_url` from launch); without a run there is nothing to link. |
| A14 | `prlink.js:360` | The gate's sub line omits the author when `pr_link.author` is null (notes F9: "unknown author"). | A line that says "opened by unknown author" adds nothing on a gate. |

## Checked and matching (no change)
- **Rows**: two-line anatomy, draft chip, bot glyph with `aria-label="bot"`, `unknown author`, `app/` stripped, CI piece only with counts, review via `reviewWord` (no "no review"), Launch team… / Open feature / disabled fork with `aria-describedby`, `Earlier team: slug · done` with the slug as a link, `Team on this PR: slug · executing`, rows dimmed and disabled while loading, `Loading pull requests from gh…` on first load, `Not loaded` / `No open pull request.`; keys ↑ ↓ Enter `l` `o`; row patched from the launch result (F8).
- **Dialog**: chip `From PR #42 · GitHub ↗` (`aria-label="Open PR #42 in GitHub"`, tooltip the title), no Feature segment, Project static, Base replaced by **Works on** (link, `head → base` mono, bot glyph, ` · draft`, mid-cut > 48, tooltip), no Pull requests control, brief label and 12 rows scrolled to the top, logs-cut line only for `logs: "cut"`, loading state (skeletons, disabled Team / Brief / Issues / off-hours / Launch, Works on filled, 8 s second line), Launch focused on success, a late prefill dropped on Cancel, the three bullets, the untrusted-text note, pre-flight warnings (Launch stays enabled), the eight prefill errors and the launch errors (fixable ones keep Launch, `pr_closed` / `pr_merged` / `fork` disable it, `active_feature` adds Open feature), `Launching…`.
- **Feature pane**: header `… · PR #42 · started …` with the real branch, the linked line (`Works on PR #42 ↗ · head → base · opened by …`), pre-push line and its gate variant with **Show card**, the head-moved warning with the sha link (no sha → "a commit"; "probably by" only for a bot), the closed line, spec gate note (D5).
- **Gate card**: `Push {n} commit(s) to PR #42`, sub `head → base · opened by …`, the exact note, diff header "Your team's changes on top of PR #42", no "Closes" line, lease line, head-moved warning while open, approve **Push**, outcomes (`Pushed 3 commits to PR #42`, toast `Pushed to PR #42` with Open PR, the three `Not pushed:` lines from `payload.outcome.code`, the core sets the outcome and fails the gate: `pr_push.rs:150-160`).
- **Delete dialog**: fourth bullet, the linked checkbox copy, the discard copy, the kept line (only when checked and the plan is known), toast `· kept {branch}: {n} commits not pushed to PR #42` (no error tone for a kept branch; `commits: null` handled).

## Test gaps (not blockers)
1. No DOM test of the linked gate card: `data-pr-note`, `data-pr-diffhead`, `data-pr-lease`, `data-pr-moved` (the copy is covered in `prlink.test.js`, the markup is not), nor of the spec gate's `data-linked-note`.
2. No test of the `o` key on a row, nor of Esc on the PR dialog (only Cancel).
3. No test of the 8 s "Still reading" line (`prs.slow`) nor of the logs-cut line (`data-logs-cut`).
4. No test of the Feature pane's head-moved warning without a sha (only with `a41d7e0`) at the component level.
