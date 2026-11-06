# Add Setting For Live Cap: API contract

Two Tauri commands in `crates/orch-app/src/commands.rs` (registered in
`lib.rs`), called from the UI with `invoke` (`ui/src/lib/ipc.js`). Errors are
plain strings, as for the other commands (`errText`).

## `pacing_settings` → `PacingSettings`

No arguments.

```json
{
  "live_cap": 3,
  "live_cap_default": 3,
  "live_cap_min": 1,
  "live_cap_max": 20,
  "off_hours_live_cap": 1,
  "off_hours_live_cap_default": 1,
  "off_hours_live_cap_min": 0,
  "off_hours_live_cap_max": 20,
  "off_hours_allowance_pct": 0,
  "shadow": true,
  "derived": {
    "green": 3, "amber": 2, "red": 1, "catch_up": 5,
    "memory_normal": 3, "memory_warn": 2, "memory_critical": 1
  },
  "now": {
    "state": "amber",
    "governor_cap": 2,
    "live_autonomous": 1,
    "pressure": "normal",
    "process_cap": 3
  }
}
```

- `live_cap`: the base in effect (setting, else default; under `orchd --cap`,
  the flag's value).
- `off_hours_allowance_pct`: from the current plan; 0 means agents don't run
  off-hours whatever `off_hours_live_cap` is (the UI says so).
- `shadow`: Governor in shadow mode (the cap is logged, not enforced).
- `derived`: the caps the base gives, same formulas as the core
  (`LiveCaps::from_base`, `resources::effective_cap`). The UI recomputes them
  while the user types with the same formulas:
  `green = b`, `amber = ceil(b/2)`, `red = min(1, b)`, `catch_up = b + 2`,
  `memory_normal = max(b, 1)`, `memory_warn = max(b − 1, 1)`,
  `memory_critical = 1`.
- `now.state`: `green | amber | red | catch_up` (as `snapshot.state`);
  `now.governor_cap` = `snapshot.live_cap` (block weight and 5h guard
  included); `now.pressure`: `normal | warn | critical` (effective level);
  `now.process_cap` = `Runtime::live_cap()`.
- Fails with the runtime's error when agents aren't running (no runtime).

## `pacing_settings_set` `{ live_cap?: u32, off_hours_live_cap?: u32 }` → `PacingSettings`

- Each field optional; absent = unchanged. Both checked before anything is
  written: `live agents {n}: use 1–20`, `off-hours live agents {n}: use 0–20`.
- Stores `governor.live_cap` / the policy's `off_hours_live_cap`, rebuilds the
  Governor once, re-syncs process slots, logs the events, pushes the usage
  refresh the UI already listens to, and returns the new view.
- A value equal to the current one is a no-op (no event, no rebuild).

## Unchanged

- `plan_form` / `plan_set` keep `off_hours_live_cap` (optional) for
  compatibility; the Week planner UI doesn't send it.
- The usage snapshot keeps `live_cap` (the Governor's cap now).
