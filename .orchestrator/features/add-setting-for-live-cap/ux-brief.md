# Add Setting For Live Cap: UX brief

## Where

Settings pane (`ui/src/panes/SettingsPane.svelte`), section **Pacing policy**
(nav item already there). It replaces the "Not designed yet" placeholder for
that section only. Same layout as the Integrations and MCP sections (FigCraft
page `04.7 Settings`, p3-design-notes §5.1): the section list on the left,
the section on the right. Draw it at 1512 and check it at 1280.

## Content (top to bottom)

1. **Heading** "Pacing policy", one line under it: how many agents may run at
   once. The Week planner edits the plan (days, hours, reserve, allocations):
   link to it.
2. **Live agents**: number field 1–20, default 3. Helper: "Autonomous agents
   running at once when the week is on plan."
3. **What it gives** (read only, updates as the user types):
   - by Governor state: Green 3 · Amber 2 · Red 1 · Catch-up 5, each with its
     pressure color/icon (`PRESSURE` in the UI, existing tokens);
   - under memory pressure: normal 3 · warn 2 · critical 1;
   - one muted line: half-weight hours scale the cap down; Leads and the
     Overwatch have their own slot and don't count.
4. **Off-hours live agents**: number field 0–20, default 1. Helper: agents
   running outside working hours while the week is a little over plan. When
   the plan has no off-hours allowance (`off_hours_allowance_pct = 0`), the
   field stays editable but a note says agents don't run off-hours, with a
   link to the Week planner.
5. **Now** (muted): "Now: Amber · 2 allowed · 1 live · memory normal".
6. **Actions**: Apply (primary, disabled until a value changed) and "Reset to
   defaults". Applying shows "Saved" inline (and a toast is fine); the "Now"
   line refreshes.

## Notes and states

- **Shadow mode on**: a note near the top: the Governor logs the cap without
  enforcing it; the memory cap still applies.
- **Above 6**: soft warning under Live agents about memory (about 190–255 MB
  per agent) and that the week is used sooner; never blocks Apply.
- **Lowering** below the agents live now: note before Apply: "Working agents
  finish their turn; new turns wait for a slot."
- **Field error** (out of range, or the core refused): under the field, the
  core's text; Apply disabled while a field is invalid.
- **Loading**, **saving** (Apply busy), **load error** (agents not running:
  the core's error text, no fields), **save error** (inline, values kept).
- Copy follows the copy rules on page `05 Keyboard & copy` (sentence case,
  no exclamation marks, numbers as digits).

## Keyboard

Same as the other sections: ⏎ from the list enters the section and focuses
Live agents; Tab moves between fields and buttons; ↑/↓ step the number;
⏎ in a field applies; esc returns to the section list. List the keys in the
section's hint line like the other sections.

## Deliverables

- UX: copy, states and keys written in
  `.orchestrator/features/add-setting-for-live-cap/design-notes.md`, handed to UI.
- UI: frames on `04.7 Settings` (default, edited with derived values, shadow
  note, above-6 warning, field error, load error), light and dark if the page
  has both, exported as PNG to
  `.orchestrator/features/add-setting-for-live-cap/design/`. No new token
  unless unavoidable.
