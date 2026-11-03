# Add Setting For Live Cap: design notes (UX)

One screen: **Settings › Pacing policy**, replacing the "Not designed yet"
placeholder in `SettingsPane.svelte`. Same frame as Integrations/MCP (nav 200 px
left, section right, p3-design-notes §5.1). Data: `pacing_settings` and
`pacing_settings_set` (api-contract.md). Copy rules: sentence case, no
exclamation marks, numbers as digits, past tense for toasts (p3 §6).

## 1. Layout (top to bottom)

```
Pacing policy                                     <- h2
How many agents may run at once. The Week planner edits the plan: days, hours, reserve, allocations.
[shadow note]                                     <- only in shadow mode

Live agents            [ 3 ]  (1–20)
Autonomous agents running at once when the week is on plan.
[above-6 warning] [lowering note] [field error]

  Green ● 3 · Amber ▲ 2 · Red ■ 1 · Catch-up ⇡ 5     <- "What it gives" block
  Memory: normal 3 · warn 2 · critical 1
  Half-weight hours scale the cap down. Leads and the Overwatch have their own slot and don't count.

Off-hours live agents  [ 1 ]  (0–20)
Agents running outside working hours while the week is a little over plan.
[no-allowance note] [field error]

Now: Amber · 2 allowed · 1 live · memory normal   <- muted

[Apply]  [Reset to defaults]   Saved               <- "Saved" inline, muted-success

hint line
```

Fields: narrow number inputs (about 72 px), label above, range after the field in
muted text. "What it gives" is a read-only bordered block under Live agents; each
Governor state uses its `PRESSURE` icon and `--color-pacing-*` color (status.js),
with the label as text (never color alone).

## 2. Exact copy

| Item | Copy |
|---|---|
| Heading | `Pacing policy` |
| Intro | `How many agents may run at once. The Week planner edits the plan: days, hours, reserve, allocations.` ("Week planner" is a link: opens the planner pane, key `p` as elsewhere) |
| Live agents label | `Live agents` |
| Live agents range | `1–20` |
| Live agents helper | `Autonomous agents running at once when the week is on plan.` |
| Derived, Governor | `Green 3 · Amber 2 · Red 1 · Catch-up 5` (each state: icon + label + number) |
| Derived, memory | `Memory: normal 3 · warn 2 · critical 1` |
| Derived, muted line | `Half-weight hours scale the cap down. Leads and the Overwatch have their own slot and don't count.` |
| Off-hours label | `Off-hours live agents` |
| Off-hours range | `0–20` |
| Off-hours helper | `Agents running outside working hours while the week is a little over plan.` |
| Now | `Now: Amber · 2 allowed · 1 live · memory normal` (state label from `PRESSURE`; "2 allowed" = `governor_cap`; "1 live" = `live_autonomous`; memory = `pressure`) |
| Apply | `Apply` (primary) |
| Reset | `Reset to defaults` (secondary; sets the fields to 3 and 1, does not save) |
| Saved | `Saved` (inline after Apply, fades after 3 s; toast `Live agents set to 6` / `Pacing settings saved` when both changed) |
| Hint line | `⏎ apply · ↑↓ step · tab next · esc back to sections` |

Derived values are recomputed as the user types, with the contract's formulas
(`green = b`, `amber = ceil(b/2)`, `red = min(1,b)`, `catch_up = b+2`,
`memory_normal = max(b,1)`, `memory_warn = max(b−1,1)`, `memory_critical = 1`).
They show the edited value, not the saved one; the "Now" line always shows the
saved state. While a field is invalid, the block keeps the last valid numbers
and dims (opacity), no error text in the block.

## 3. States

| State | What shows |
|---|---|
| **Default** | Saved values (3 and 1 on a fresh install). Apply disabled. Derived `Green 3 · Amber 2 · Red 1 · Catch-up 5`, memory `3 · 2 · 1`. "Now" line. |
| **Edited** | A field differs from the saved value: Apply enabled, derived block updates live, fields with a change get the standard "modified" mark used by other settings (no new token). "Reset to defaults" enabled when a value is not the default. |
| **Shadow mode** | Note under the intro, info style (muted icon + text): `Shadow mode is on: the Governor logs the cap without enforcing it. The memory cap still applies.` The Governor row of the derived block is muted with the suffix `logged only`; the memory row stays normal. |
| **Above 6** (Live agents > 6) | Warning under the field (warning color + glyph): `More than 6 agents can use 190–255 MB each and use the week sooner. The Governor still paces the week.` Never blocks Apply. |
| **Lowering** (new value < `now.live_autonomous` or < the saved value while agents are live) | Note above Apply, muted: `Working agents finish their turn; new turns wait for a slot.` Shown only while the edit is below the live count. |
| **No off-hours allowance** (`off_hours_allowance_pct = 0`) | Field stays editable. Note under it: `The plan has no off-hours allowance, so agents don't run off-hours. Set one in the Week planner.` ("Week planner" is a link.) |
| **Field error** | Under the field, error color + glyph, the core's text verbatim: `live agents 0: use 1–20`, `off-hours live agents 25: use 0–20`. Checked on every keystroke (client mirror of the bounds); the core's text is shown if the set call refuses. Apply disabled while any field is invalid; the empty field counts as invalid with `live agents: use 1–20`. Non-integers are refused the same way. |
| **Loading** | Heading and intro show. Fields and blocks as skeleton rows (two fields, one block); header line `Reading the pacing settings…`. Nothing is editable. |
| **Saving** | Apply shows busy (`Applying…`), fields and Reset disabled, no layout shift. |
| **Saved** | `Saved` next to the buttons, Apply disabled again, "Now" line refreshed from the returned view, derived block matches. |
| **Load error** | No fields. Error line: `Could not read the pacing settings: {core text}` (e.g. agents not running) and a `Retry` button (`r`). |
| **Save error** | Inline above the buttons, error style: `Could not save: {core text}`. Values kept as typed, Apply stays enabled so the user can retry. |

Notes:
- The shadow note and the load error are the only things that replace content.
  Everything else stacks under its field in this order: error > above-6 warning >
  lowering note (the lowering note sits above Apply, not under the field).
- `--cap N` runs: the value shown is the flag's. No extra UI; if it differs from
  the stored one after Apply, the returned view wins.

## 4. Keyboard

| Key | Where | Action |
|---|---|---|
| `⏎` | nav list | Enter the section, focus **Live agents** (`data-section-focus`) |
| `tab` / `⇧tab` | section | Live agents → Off-hours live agents → Apply → Reset to defaults (the Week planner links come after their field) |
| `↑` / `↓` | number field | Step the value by 1 (stops at the bounds) |
| `⏎` | in a field | Apply (no-op while Apply is disabled) |
| `esc` | in a field | Leave the field, focus stays in the section; a second `esc` returns to the section list (already handled by `onBodyKey`: not from inside an input) |
| `esc` | elsewhere in the section | Back to the section list |
| `r` | load error | Retry |

No new global key; none conflicts with the existing Settings keys (`j/k`, `⏎`,
`esc`). `j/k` don't act inside the section (typing in a number field).

Hint line (muted, bottom of the section, like Integrations):
`⏎ apply · ↑↓ step · tab next · esc back to sections`

## 5. Checklist against ux-brief.md

Heading + intro with link ✓ · Live agents + helper ✓ · derived by state and
memory + muted line ✓ · Off-hours + helper + no-allowance note ✓ · Now ✓ ·
Apply / Reset / Saved ✓ · shadow ✓ · above 6 ✓ · lowering ✓ · field error ✓ ·
loading / saving / load error / save error ✓ · keyboard + hint line ✓.

## 6. For ui

Frames on `04.7 Settings` at 1512 (check 1280): default, edited (derived
updated), shadow note, above 6, field error, load error; light and dark if the
page has both. Tokens: `--color-pacing-*` for the state icons, existing error and
warning tokens, no new token.
