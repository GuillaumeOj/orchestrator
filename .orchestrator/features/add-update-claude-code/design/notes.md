# Update Claude Code · design notes (T-243, ui)

Frames: FigCraft [`Orchestrator.figcraft`](https://drive.google.com/open?id=1n4O57ntQDFvv1-pw3flJkmcHh4MeTLVs&usp=drive_fs) (Google Drive, not in the repo), page **04.7 Settings**, layers `claude-update-01…09` (spanning x 0–6800, y 4700–7082). Exports here, 1280 px wide, dark unless noted. Copy and states follow [ux-brief.md](../ux-brief.md) §2; this file only records the visual choices.

| Frame | Shows |
|---|---|
| 01-up-to-date | Settings › Claude Code, up to date; no nav badge, no Claude chip (app chip only) |
| 02-available | accent `↑` nav badge, chip `↑ Claude Code 2.1.330` left of the app chip with its tooltip, primary button focused, amber "newer than last tested" note |
| 03-updating | spinner in place of the badge, muted chip, no buttons, 12-line log, hints `esc` only |
| 04-updated | green dot, the "agents already running…" line, `Show output` link, amber compat note, success toast |
| 05-failures | A: update failed (amber dot, amber chip, log open "Output of claude update" + `Hide output`, failure toast with `Details`). B: check failed (no badge, no chip) |
| 06-readonly-banner | idle / updating / update failed (the failed variant shortens the core's message only to leave the added clause room in the frame; the real banner keeps the core's text and cuts the whole line with an ellipsis) |
| 07-setup-step-1 | Setup step 1, version problem, idle and updating (log drawn open to show the 6-line height; v1 shows it only on failure) |
| 08-edge-states | not found (the core's `problem` text, wrapped), never checked + start-up pending, Homebrew / unknown hints, Too old (red note plus the amber `latest` note), updated-then-newer, palette group |
| 09-available-light | frame 02 in the light theme |

Visual rules (existing tokens only, no new component):
- **Blocks**: header and update blocks reuse Builds' `.run` box (`surface.pane`, `border.subtle`, `radius.md`, 8/12 padding). The update block switches to `surface.raised` when it has a primary action or a failure (available, update failed), so the block that asks for attention reads first.
- **State text**: 13 px; the lead words semibold, the rest regular. Dot / glyph colours: up to date `pacing.green`, available `accent.default`, updating `attention.running` (spinner `◐`), failures `pacing.amber` with `▲`, not found `pacing.red`. Second lines: `text-caption` (11 px) muted, indented under the text (past the dot).
- **One line**: the state line is cut with an ellipsis before the buttons (full error in the tooltip); second lines may run under the buttons, which then sit on the first line, and wrap inside the block when longer than it (04).
- **Buttons**: right-aligned, primary rightmost; the ⏎ default gets the focus ring and an inline `⏎` key cap. Disabled = 40 % opacity.
- **Log**: `surface.chrome` panel, `border.subtle`, mono 12 px / 16 px lines, error lines `pacing.amber`; label row above (11 px muted) with the `Hide output` link right-aligned.
- **Chip**: the status-bar `.chip` (18 px, full radius). available: border and text `accent.default`; updating: border `border.subtle`, text muted, spinner; failed: border and text `pacing.amber`. 8 px before the app chip.
- **Nav badge**: right edge of the entry, 8 px dot (`status-dot`) or the `↑` / `◐` glyph at 13 px.
- **Banner**: unchanged `Banner.svelte` row; the text is cut with an ellipsis before the buttons. In the failed variant the added clause is `pacing.amber`.
- **Toasts**: existing toast; 4 px left bar `pacing.green` (success) or `pacing.amber` (failure).
