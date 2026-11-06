# Update Claude Code · UX brief

> For ux (flows, states, copy) then ui (FigCraft frames, exported to `design/`). Inputs: [spec.md](spec.md), [api-contract.md](api-contract.md). Reuse the Settings › Builds section's patterns (`BuildsSettings.svelte`: header block, state line, inline log/confirm, muted footnotes) and the existing tokens; no new components unless one is clearly missing.

## 1. Who and when

The user runs several agents all day. Claude Code ships often (several times a week). They want to know a new version exists **without being nagged**, and to apply it in one click when it suits them, without opening a terminal or restarting the app. Updating must never feel like it could kill running agents (it doesn't: they keep their version until they restart).

## 2. Surfaces

### 2.1 Settings › Claude Code (main surface)

Section content, top to bottom (ux decides the exact layout):

1. **Header**: `Claude Code 2.1.317` · install method (`native installer` / `Homebrew` / `npm` / `unknown install`) · channel (`latest` / `stable`). Path below in mono, `~`-shortened (copyable).
2. **State line + actions**, one of:
   | state | line | actions |
   |---|---|---|
   | checking | `Checking for updates…` | — |
   | up to date | `Up to date · checked 14:02` | Check now |
   | available | `2.1.330 is available` (accent) | **Update to 2.1.330** (primary) · Check now |
   | updating | `Updating to 2.1.330…` + elapsed, with the live log (≤ 40 lines, mono, scrolls to bottom) | — (no cancel in v1) |
   | updated | `Updated 2.1.317 → 2.1.330. New and resumed agents use it; running agents keep 2.1.317 until they restart.` | Check now |
   | check failed | `Couldn't check for updates: <error>` (amber) · last known latest if any | Check now · Update anyway (secondary: runs `claude update`) |
   | update failed | `Update failed: <error>` (amber) + the log, open | Retry · Check now |
   | never checked / offline | `Not checked yet` | Check now |
3. **Compatibility**: `Tested up to 2.1.302 · minimum 2.1.302`. When the installed (or the offered) version is above the tested max: an amber note, "Newer than the last tested version: agents run, with a warning. Run the contract tests." When below the minimum: the red read-only state (2.4).
4. **Account** (read-only, existing data): logged in · `claude.ai` · `max`. The `note` (login could not be checked) in muted text.
5. **Footnote** (muted): "Agents run with auto-update off. The app updates Claude Code only when you ask, and never writes to ~/.claude."

Install-method hints (muted, under the state line, only when relevant): Homebrew → "Homebrew installs update through brew; this may take a while." `unknown` → "If the update fails, update Claude Code the way you installed it."

Keyboard: follows Settings (esc back to nav). In the section: `u` = Update (when offered), `r` = Check now; the primary button takes focus on ⏎ into the section. Show the hints like the other sections.

Nav: the "Claude Code" entry gets a badge when an update is available (accent, `↑`) or a `problem` is set (error).

### 2.2 Status bar chip

Next to the Orchestrator updater's chip; same sizes, distinguishable (it must say *Claude Code*, not just a version, so it is never mistaken for the app's own update).
- available: `Claude Code 2.1.330` with an up mark; tooltip "Claude Code 2.1.330 is available (you run 2.1.317). Click to open Settings." Click → Settings › Claude Code.
- updating: `Updating Claude Code…` (plain).
- update failed: `⚠ Claude Code update` (amber); click → Settings.
- otherwise nothing. A check failure alone shows nothing (no nagging when offline).
No toast for "available". One toast when an update finishes: "Claude Code updated to 2.1.330." / "Claude Code update failed" (amber, with "Details" → Settings).

### 2.3 Palette

"Check for Claude Code updates" (toast with the outcome, like the app's "Check for updates") and "Update Claude Code" (greyed with a hint: "no update available" / "an update is running").

### 2.4 Read-only banner and Setup step 1

When `problem_kind == "version"` (Claude Code too old): the red "Read-only:" banner keeps its text (now "… update Claude Code (Settings › Claude Code, or claude update).") and gets an **Update Claude Code** button before "Set up". While it runs the button reads "Updating…" (disabled). On success the banner goes away by itself (the app leaves read-only). Setup step 1 gets the same button and shows the same state line as 2.1 inline.

## 3. Frames for ui (FigCraft, exported PNGs in `design/`)

1. Settings › Claude Code: up to date.
2. Same: update available (+ nav badge, + status bar chip visible).
3. Same: updating, with the log.
4. Same: updated (with the "running agents keep…" line) and untested amber note.
5. Same: update failed (log open) and check failed (two compact variants, one frame).
6. Read-only banner with "Update Claude Code" (top of the app).
Dark theme first; one light variant of frame 2.
