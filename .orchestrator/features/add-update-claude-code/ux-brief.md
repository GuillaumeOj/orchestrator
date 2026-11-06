# Update Claude Code · UX brief

> **Final (T-242, ux).** Sections marked ▲ were added or changed in this pass. For ui (FigCraft frames, exported to `design/`). Inputs: see below; was: ux (flows, states, copy) then ui (FigCraft frames, exported to `design/`). Inputs: [spec.md](spec.md), [api-contract.md](api-contract.md). Reuse the Settings › Builds section's patterns (`BuildsSettings.svelte`: header block, state line, inline log/confirm, muted footnotes) and the existing tokens; no new components unless one is clearly missing.

## 1. Who and when

The user runs several agents all day. Claude Code ships often (several times a week). They want to know a new version exists **without being nagged**, and to apply it in one click when it suits them, without opening a terminal or restarting the app. Updating must never feel like it could kill running agents (it doesn't: they keep their version until they restart).

## 2. Surfaces

### 2.1 Settings › Claude Code ▲

Single column, same width and rhythm as Settings › Builds (`h2` header, muted lines, `btn` / `btn primary` / `link`). Top to bottom:

1. **Header block** (like Builds' "Running" block): label `Installed` (muted), then `2.1.317` (mono, large as Builds' version) · `native installer` · `channel latest`. Line 2 (muted, mono, `~`-shortened path, click copies, tooltip "Copy path"). Install names: `native installer` / `Homebrew` / `npm` / `unknown install`. Version unknown (not found) → see "Not found" below.
2. **Update block** (the only block with a primary action). Left: state dot + text; right: buttons. States, first match wins in this order: not found › updating › update failed › check failed › checking › updated › available › up to date › never checked.

   | state | when (`setup.claude`) | text (tone) | buttons (⏎ = focused default) |
   |---|---|---|---|
   | not found | `problem_kind == "not_found"` | `Claude Code not found.` (red) + the `problem` text | none; link "Set up" |
   | updating | `update.state == "updating"` | `Updating to 2.1.330…  0:42` (elapsed from `started_at`, ticks each second, tabular figures); log open below | none, no cancel in v1 |
   | update failed | `state == "error"` and `error_from == "update"` | `Update failed: <error>` (amber, one line, ellipsis, full text in tooltip); log open below, labelled "Output of claude update" | **Retry** `⏎` · Check now |
   | check failed | `state == "error"` and `error_from == "check"` | `Couldn't check for updates: <error>` (amber). If `latest` is known and `available`: second muted line `Last known: 2.1.330 (checked 14:02)` | Check now `⏎` · Update anyway (secondary) |
   | checking | `state == "checking"` | `Checking for updates…` (muted, spinner as Builds' refreshing) | none |
   | updated | `update.updated` set, `state == "idle"`, and not `available` | `Updated 2.1.317 → 2.1.330.` (green dot) + muted second line `New and resumed agents use it; running agents keep 2.1.317 until they restart.` Log collapsed behind a `link` "Show output". | Check now |
   | available | `available` | `2.1.330 is available` (accent dot, accent text) | **Update to 2.1.330** `⏎` (primary) · Check now |
   | up to date | `latest` known, not `available` | `Up to date · checked 14:02` (`checked` omitted if `checked_at` null; "yesterday 22:10" style as `when()`) | Check now |
   | never checked | `latest == null`, no error | `Not checked yet` (muted) | Check now `⏎` |

   - `updated` outranks `available` only when the check after the update found nothing newer; a still-newer release shows `available` and keeps the "Updated …" line as a muted second line above it.
   - `updated` stays until the next check or update begins (it also disappears at app restart; the contract does not persist it).
   - `Update anyway` is shown only for check failures; it runs `claude_update_run` exactly like Update.
   - Buttons disabled while another command is in flight (checking, updating); never hidden, except where the table says none.
3. **Log** (only for updating, update failed, and the expanded `updated`): mono, 12 px, max 40 lines, panel height ≈ 10 lines with its own scroll, sticks to bottom while updating unless the user scrolled up, `user-select: text`. Empty log while updating: `Waiting for output…` (muted). Update failed: log open by default, "Hide output" link.
4. **Hints** (muted, one line, only when relevant, directly under the update block): Homebrew → `Homebrew installs update through brew; this may take a while.` (also while updating). `unknown` → `If the update fails, update Claude Code the way you installed it.` `npm` → none.
5. **Compatibility** (muted two-column rows, label / value): `Supported` · `2.1.302 (minimum) – 2.1.302 (last tested)`. Notes below, only when they apply:
   - installed or `latest` above `tested_max`: amber note, `Newer than the last tested version. Agents run, with a warning.` (two cases, same copy; for `latest` only: prefix `2.1.330 is newer than the last tested version. ` and keep the Update button available, the note is not a blocker.) Drop "Run the contract tests" (dev-only wording; the note links nowhere in v1).
   - installed below `min`: red note `Too old. The app is read-only until Claude Code is updated.` (matches `problem_kind == "version"`).
6. **Account** (read-only rows): `Login` · `Logged in · claude.ai · max` or `Not logged in`; the `note` muted below. Rows reuse Builds' label/value style.
7. **Footnote** (muted): `Agents run with auto-update off. The app updates Claude Code only when you ask, and never writes to ~/.claude.`

**Keyboard** (section focused; all single keys, shown in a hints line like Builds: `u update · r check now · esc back to sections`):
- `⏎` into the section from the nav focuses the section; `⏎` inside triggers the block's default button (the one marked ⏎ above).
- `u`: Update / Retry / Update anyway, whichever is shown (no-op otherwise, silent).
- `r`: Check now (no-op while checking or updating).
- `esc`: back to the nav (a log scroll area never traps esc).
- Hint line shows only keys that are live in the current state.
- Focus rule: after Update starts, focus stays on the section (not on a vanished button) so `esc` still works; on completion nothing steals focus. `aria-live="polite"` on the state text.

**Nav badge** ▲ on the "Claude Code" entry, right-aligned, one at a time: error dot (red) if `problem` is set; else amber dot if `update.state == "error"` (failed update only, `error_from == "update"`; a failed check never badges); else accent `↑` if `available`; else none. While updating: spinner glyph in place of the badge. Clears when the cause clears; it does not clear by visiting the section.

### 2.2 Status bar chip ▲

Left of the app updater's chip, 8 px gap, same height and type. Always carries the words *Claude Code* so it is never mistaken for the app's update. Priority, one chip only: updating › update failed › available › nothing.
- available: `↑ Claude Code 2.1.330` (accent text on the raised chip). Tooltip: `Claude Code 2.1.330 is available (you run 2.1.317). Click to open Settings.` Click → Settings › Claude Code (select the section; does not start the update).
- updating: `Updating Claude Code…` (muted, spinner, not clickable-looking but click still opens Settings; tooltip `Running claude update. Click for the output.`).
- update failed: `⚠ Claude Code update` (amber); tooltip `Claude Code update failed: <error>. Click for details.`
- when the app updater also shows a chip, both show; the Claude chip is first. At narrow widths the Claude chip drops its version (`↑ Claude Code`) before the app chip truncates.
- A failed check shows nothing (offline stays quiet); the same for `problem_kind == "version"`: the red banner already carries it, no chip.

**Toasts** (only for runs of this session, i.e. when `update.updated` changes or `state` goes `updating → error`; never at start-up, never for "available"):
- success: `Claude Code updated to 2.1.330.` (3 s, no action).
- failure: `Claude Code update failed.` (amber, stays until dismissed, action `Details` → Settings › Claude Code).
- Palette-triggered check: `Claude Code is up to date.` / `Claude Code 2.1.330 is available.` / `Couldn't check for Claude Code updates: <error>`.

### 2.3 Palette ▲

Group "Claude Code", two commands:
- `Check for Claude Code updates` (disabled with hint `an update is running` / `Claude Code not found`).
- `Update Claude Code` (enabled only when `available`; otherwise disabled with hint `no update available` / `an update is running` / `Claude Code not found`). Running it from the palette does the same as the button and shows the chip + toast; it does not navigate.
Neither takes a key chord in v1.

### 2.4 Read-only banner and Setup step 1 ▲

Only when `problem_kind == "version"`. The red banner text becomes the core's message (`… update Claude Code (Settings › Claude Code, or claude update).`) and gets **Update Claude Code** (plain `btn`, not primary: the banner row already has "Set up") placed before "Set up". Per state:
- idle / available / check failed: `Update Claude Code`. If `latest` is known and not newer than the installed version (a pinned package manager), the button still shows: the user may try; failures read below.
- updating: `Updating…` (disabled, spinner). Banner text unchanged.
- update failed: button `Retry update`; the text gets one more clause: ` Update failed: <error>` (cut to one line; the full output lives in Settings, and the banner's `Set up` is replaced by `Details` → Settings › Claude Code).
- success: the banner disappears by itself when the problem clears. Toast as 2.2. If after the update the version is still too old, the same `Update failed` path applies (core reports "still 2.1.x").
- Other `problem_kind`s (not_found, login, plan): no update button (an update won't fix them).
- Dismissing the banner (×) hides it for the session as today; the update then continues unaffected.

Setup step 1 (version problem): the row `path · version` gets, on the right, the same button, and below it the **same state line as 2.1.2** (reuse the component/helper, compact: no log by default; on failure show the log open, 6 lines high, plus `Open Settings › Claude Code`). In the non-problem case (up to date or untested) step 1 is unchanged except that `untested` keeps its existing note.

## 3. Frames for ui (FigCraft, exported PNGs in `design/`) ▲

Dark theme first, 1280×800 app frames with the Settings pane in view unless stated; name files `claude-update-NN-<name>.png`.
1. `01-up-to-date`: Settings › Claude Code, up to date; nav entry without badge; no chip.
2. `02-available`: update available; nav badge `↑`; status bar chip `↑ Claude Code 2.1.330` beside the app chip (show a second app-updater chip too); primary button focused.
3. `03-updating`: log filling (12–15 lines), elapsed `0:42`, nav spinner, chip `Updating Claude Code…`, buttons absent.
4. `04-updated`: `Updated 2.1.317 → 2.1.330.` with the "running agents keep…" line, "Show output" link, plus the amber "newer than last tested" note; no chip.
5. `05-failures`: one frame, two compact states stacked: update failed (log open, Retry + Check now, nav red/amber dot, chip `⚠ Claude Code update`) and check failed (`Last known: …`, Check now + Update anyway).
6. `06-readonly-banner`: top-of-app strip, three variants stacked: idle (`Update Claude Code` + `Set up`), updating (`Updating…`), update failed (`Retry update` + `Details`).
7. `07-setup-step-1`: Setup pane step 1 with the version problem and the inline state line (idle and updating).
8. `08-edge-states`: Settings pane, compact stack: not found, never checked, Homebrew hint, "Too old" red compat note.
9. `09-available-light`: light variant of frame 2.
Palette entries (2.3) are text: ui draws them once, in the open palette, in frame 2 if space allows; otherwise skip.
