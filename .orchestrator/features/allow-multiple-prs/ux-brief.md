# Allow multiple PRs · UX brief

> Companion of [spec.md](spec.md) and [api-contract.md](api-contract.md). UX turns this into flows and wireframes; UI draws the frames in the FigCraft file (Drive `Orchestrator.figcraft`) and exports them under `design/`. Reuse the existing tokens and components (PrLine, chips, segmented controls, decision cards); no new colors.

## 0. Principles

- **One PR stays the default.** One PR per issue is an explicit choice of the user, never a side effect of listing issues.
- The user sees, per issue, where its PR is: none yet, waiting for approval, open (CI, position), merged, closed.
- The same PR line (`PrLine`, improve-end-of-feature ux-brief §1) is reused per issue; only an issue label is added in front.

## 1. Launch form (Dialogs.svelte, team launch)

Placed after the brief, before "Run off-hours".

### 1.1 Issues field
- Label **Issues** (optional). A chip input: type `12`, `#12`, `ENG-7`, a GitHub issue URL; Enter, comma or space makes a chip. Chip text = the ref; its title (when known) muted after it, truncated. `×` removes it.
- Suggestions under the field while typing (`issue_suggest`): ref + title rows, from the project's intake. Arrow keys / Enter.
- Invalid entry: the chip in error style, the message under the field: `Not an issue of <owner/repo>: <text>` / `Not an issue ref: <text>`. Launch disabled while one chip is invalid.
- Prefilled with the intake item's ref when started from an item.
- At 10 chips the input disables: `10 issues at most`.

### 1.2 Pull requests choice
- Label **Pull requests**. Segmented control: **One PR** (default) · **One PR per issue**.
- "One PR per issue" disabled under 2 issues, tooltip: `Add at least 2 issues`.
- Helper line under it (muted), by choice:
  - One PR, no issue: `One PR for the feature.`
  - One PR, issues: `One PR for the feature; it closes #12, ENG-7.`
  - One PR per issue: `One PR per issue, each built from main with that issue's tasks. Work not tied to an issue goes into every PR.`
- Removing chips down to 1 while "per issue" is selected switches back to One PR (no message needed, the helper changes).

## 2. Feature pane

### 2.1 Issues and mode (header area or the "PR and artifacts" card head — UX decides)
- Read view: `Issues #12 · ENG-7` (each a link to its url when known) and `One PR` / `One PR per issue`; an **Edit** button.
- Edit: the same chip input and segmented control as the launch form, in a popover, **Save** / **Cancel** (`feature_set_prs`).
- Locked parts, with the reason as tooltip / inline text:
  - an issue with a PR (pending, open or merged): its `×` hidden, tooltip `Has PR #34`;
  - the mode while a PR is pending or open: control disabled, `A PR is open: change it once no PR is open or waiting.`;
  - run done: no Edit button.
- After a save during a run: a short confirmation `Saved; the lead is told.`
- Server errors (api-contract §3) shown inline in the popover.

### 2.2 "PR and artifacts" card
- single mode: unchanged (one PrLine).
- per_issue mode: one row per issue, in the issues' order:
  - `#12 Crash on empty brief` (ref bold, title muted, truncated first) then the state:
    - `none` → muted `No PR yet` (+ task count of that issue: `· 3 tasks`, optional);
    - `pending` → `Waiting for your approval` with a link to the gate card;
    - `open` → the PrLine (`PR #34 [CI] [position]`, popover, refresh);
    - `merged` → `PR #34 merged` (done color), link;
    - `closed` → `PR #34 closed` (muted), link.
  - The refresh button checks all PRs (one button on the card head, not per row).
- Truncation order at min width: title, then position text, then CI count; the ref, PR number and state glyphs always stay.

## 3. Decision cards (Inbox / chat)

- `push_pr`, per_issue: title `Open PR for #12 "<title>"`, sub-line `orch/<slug>/issue/12 → main`. Diff list as today (the issue branch's). A muted note: `Built from the integration branch: tasks of #12 and shared work.`
- `push_pr_update`: title `Push to PR #34 (#12)`; force variant line unchanged.
- single mode cards unchanged; when issues are listed, the card shows `Closes #12, ENG-7` under the title.

## 4. Mission run row and run inspector

- single: unchanged (`prText(run.pr)`).
- per_issue compact summary (pure helper from `prs_view`): `2 PRs open · 1 merged`, `1 of 3 merged`, `3 PRs merged`; the CI dot shows the worst open PR (error > running > waiting > ok). Click opens the feature pane (not a single URL).
- Run inspector: the list of §2.2 rows (compact).

## 5. Board and task inspector

- A task with an issue shows a small chip `#12` on its card and in the inspector header. Untagged tasks of a per_issue feature show nothing (no "shared" noise).

## 6. Copy (timeline)

- `feature.prs` → `You set one PR per issue (#12, ENG-7)` / `You set one PR for the feature` / `You changed the issues: #12, ENG-7`.
- `pr.build` failed → `PR for #12 can't be built: <task handle> conflicts in <files>`.
- `pr.opened` etc. gain `(#12)` after the PR number.
- `run.state` → `every issue PR merged` / `no PR open`.

## 7. Keyboard and accessibility

- Chip input: Backspace on empty input removes the last chip; each chip's `×` is a button with `aria-label="Remove #12"`.
- Segmented control: radio group semantics, arrow keys.
- Disabled controls keep their reason reachable (tooltip on focus).

## 8. States to draw (UI checklist)

1. Launch form: no issue · 2 issues + One PR · 2 issues + One PR per issue · invalid chip · suggestions open · per-issue disabled (1 issue).
2. Feature pane per_issue card: rows in each state (none, pending, open with CI error + behind, merged, closed) in one frame; min width.
3. Issues/mode edit popover: editable · mode locked (PR open) · issue locked chip.
4. Decision cards: push_pr per issue · push_pr_update per issue · single with Closes.
5. Mission run row: per_issue summary (2 open · 1 merged, red dot).
6. Board card with issue chip.
