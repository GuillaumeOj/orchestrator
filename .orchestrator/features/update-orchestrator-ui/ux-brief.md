# Update Orchestrator UI · UX brief

> Companion of [spec.md](spec.md). The design is done: this feature draws nothing new by default. This brief says how the existing design is used and reviewed.

## Where the design is

- Frames: `.orchestrator/features/improve-orchestrator-ui-ux/design/*.png` (on main). The Drive `Orchestrator.figcraft` holds the source frames in the "Improve UI UX" rows at the top of pages 02–06; never the repo `docs/design/Orchestrator.figcraft`.
- Rules: `design-notes.md` (behaviour; §12 and §13 override earlier sections), `ux-brief.md`, `settings-brief.md`, `audit.md` §8 (actions per screen), `flows.md` (walkthroughs).
- Precedence: the PNG wins for looks (spacing, colour, copy), the notes win for behaviour. Sample data in frames is illustrative.

## Design rows that do not ship (spec §3.1)

These are drawn but **not rendered** in the implementation: Claude Code › Defaults for new sessions; Terminal & editor › Renderer, Editor command, Reuse the open nvim, New shell opens in; Memory › Show in the status bar; Project › General › Remove merged worktrees automatically. A reviewer does not flag their absence. The section reads as designed without them (no empty card, no gap).

## Review by `ui` (on front's request, per task)

Check the changed screens against the PNGs named in design-notes §1 / §13:

1. Tokens only (`docs/design/tokens.json`), no ad-hoc colours or sizes.
2. Control vs information: no `Kbd` next to or in a control; tooltips `Label · key`; icon buttons have a tooltip and `aria-label`; danger icons in `color.danger`.
3. One home per action (audit §8): nothing offered twice on one screen.
4. Select regular = text-input height; compact only in the tab strip and dock strip (R11).
5. Copy as drawn (labels, empty states, error texts); the Revert label on every footer.
6. States: the frame's state variants (hover, focused, disabled with reason, error, empty, overflow) exist.

Findings go back to front as a list *screen → PNG → deviation → expected*. A deviation where the code is right and the PNG is wrong (behaviour) goes to lead.

## Acceptance walkthrough by `ux` (T9)

For each flow F1–F14 in `flows.md`, walk the storyboard `06-flow-<id>.png` through the code and tests of the integration branch, and audit §8 per surface. Report per flow: passes / deviation (file:function, frame, rule, expected). Do not fix code; lead routes each deviation.
