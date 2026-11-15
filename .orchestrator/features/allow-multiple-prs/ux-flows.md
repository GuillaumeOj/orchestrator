# Allow multiple PRs · UX flows

> Companion of [ux-brief.md](ux-brief.md). Wireframes: FigCraft page **"allow-multiple-prs · UX"** in `Orchestrator.figcraft` (Drive); PNG exports in [design/](design/): `ux-launch-form`, `ux-feature-pane`, `ux-edit-popover`, `ux-decision-cards`, `ux-run-row-board`. The wireframes are low-fidelity (Tokyo Night palette, mono font, text-heavy): UI redraws them with the real tokens and components (PrLine, chips, segmented control, decision card).

## Decisions

1. **Where issues and mode live: a third row (`row3`) of the feature header**, under the meta line, not in the "PR and artifacts" card.
   - `FeaturePane.svelte` has `row1` (slug, phase pill, actions) and `row2` (meta). The PR card is in the right column (~320px): chips + mode + Edit do not fit, and the card only exists as "No PR yet" before the first PR, while issues and mode are settings that matter in every phase.
   - Read view: `Issues #12 · ENG-7 · One PR per issue [Edit]`. Refs link to their url when known. It truncates like row2 (`ifCut` tooltip): the mode label always stays, the refs ellipsize.
   - No issue: nothing in single mode, except a muted `Add issues` link (opens the same popover) while the run is not finished. Run done: no Edit and no link.
2. **Edit popover** hangs under the Edit button; same chip input and segmented control as the launch form. Footer: Cancel / Save; after a save during a run, `Saved; the lead is told.` for 4 s, then it closes. Server errors inline in red above the buttons.
3. **Locks are visible, not only tooltips**: a locked issue chip has no `×` and a lock glyph (`Has PR #34` as tooltip and as `aria-label`); Backspace on an empty input skips locked chips. The mode control shows the reason as a muted inline line.
4. **PR card per_issue**: ref bold + title muted, state on the same line when there is room, else on a second line. Card head shows a count summary (`2 open · 1 merged`) and the single refresh button. States: none (muted, `· 3 tasks` optional), pending (warning color, link to the gate), open (PrLine with the ref in front), merged (done color), closed (muted).
5. **Truncation at min width**: title first, then the position text (keep `↓` + number), then the CI count; ref, PR number and state glyphs always stay; full text in a tooltip.
6. **Decision cards**: the title carries the issue, the sub-line the issue branch; single mode with issues shows `Closes #12, ENG-7` under the title (the feature's issues, even when the lead's body already closes them). One gate per issue; gates of different issues wait side by side.
7. **Mission run row**: summary text from `prs_view` (`2 PRs open · 1 merged`, `1 of 3 merged`, `3 PRs merged`, `1 PR open`, `no PR yet`); dot = worst open PR (error > running > waiting > ok), none when only merged PRs remain. Click opens the feature pane; tooltip lists one line per PR. The run inspector shows the compact PR card rows.
8. **Board chip**: `#12` (ref only, mono, muted border) after the model chip; title in the tooltip; the same chip in the task inspector header. Untagged tasks show nothing. Truncation: handle > agent > issue chip.
9. **Launch form extras**: a 10-issue state (input disabled, `10 issues at most`); the suggestion list never proposes an already-added issue; with no match the typed text still becomes a chip on Enter (validated on the server).

## Flows

- **Launch, per issue**: type refs → chips (invalid ones turn red, Launch disabled) → second chip enables `One PR per issue` → pick it → helper explains the build → Launch.
- **Drop to one issue**: remove chips down to 1 while `per issue` is selected → mode silently falls back to One PR, helper text changes.
- **Mid-run switch**: Edit → mode control enabled only when no PR is open or waiting → Save → `Saved; the lead is told.`
- **PR life per issue**: none → pending (gate in Inbox) → open (CI, position) → merged / closed; a closed one may go back to none → pending when the lead requests it again.

## Copy changes

No copy of ux-brief §1–§6 changed. Added copy: `Add issues` (link), `Has PR #34` is also the `aria-label` `#12 has PR #34, cannot be removed`, the mission summary `1 PR open` / `no PR yet`.

## ux-brief.md edits

- §2.1 title: records the decision (header row3).
- §2.1 locked mode: the reason is shown inline, not only as a tooltip.

## Coverage of ux-brief §8

| §8 item | Wireframe |
|---|---|
| 1 launch states (no issue, 2+One PR, 2+per issue, invalid, suggestions, per-issue disabled) | `ux-launch-form` frames 1–6 (+7 cap) |
| 2 PR card rows + min width | `ux-feature-pane` (card, min-width frame) |
| 3 popover editable / mode locked / issue locked | `ux-edit-popover` |
| 4 decision cards | `ux-decision-cards` |
| 5 mission row summary | `ux-run-row-board` |
| 6 board card chip | `ux-run-row-board` |
