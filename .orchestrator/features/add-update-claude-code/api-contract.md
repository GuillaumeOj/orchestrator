# Update Claude Code · API contract (core ↔ UI)

Owner of the shapes: back. front codes against this; changes go through lead.

## 1. `app_state.setup.claude` (topic `setup`)

Existing fields unchanged (`path`, `version`, `untested`, `logged_in`, `auth_method`, `subscription`, `problem`, `note`). `setup.claude` stays `null` until the start-up check lands. New fields:

```jsonc
{
  "problem_kind": "version",          // null | "not_found" | "version" | "login" | "plan" — set iff `problem` is
  "install": "native",                // "native" | "homebrew" | "npm" | "unknown"
  "channel": "latest",                // "latest" | "stable" (from ~/.claude/settings.json autoUpdatesChannel)
  "supported": { "min": "2.1.302", "tested_max": "2.1.302" },
  "update": {
    "state": "idle",                  // "idle" | "checking" | "updating" | "error"
    "latest": "2.1.330",              // null until a check succeeded; kept when a later check fails
    "available": true,                // latest > version (false when either is unknown)
    "checked_at": 1793000000,         // unix secs of the last successful check, or null
    "error": null,                    // string when state == "error" (check or update failure)
    "error_from": null,               // null | "check" | "update": which one failed
    "started_at": null,               // unix secs, while state == "updating"
    "updated": { "from": "2.1.317", "to": "2.1.330", "at": 1793000100 },  // last successful update this run, or null
    "log": ["Checking for updates…", "Successfully updated from 2.1.317 to 2.1.330"]
                                      // last ≤ 40 lines of the running / last `claude update` (ANSI stripped); [] before any
  }
}
```

Rules:
- `state` returns to `idle` after a successful check or update; `error` stays until the next check or update starts.
- While `updating`, the Setup topic is touched at most every 500 ms as lines arrive, and once at the end (after the re-check, so `version`, `untested`, `problem` are already the new ones).
- `updated` is not persisted: it is gone after an app restart.

## 2. Commands (Tauri `invoke`)

| Command | Args | Returns | Errors (rejected promise, string) |
|---|---|---|---|
| `claude_update_check` | — | the new `setup.claude` object, after the check settled (≤ 20 s) | `"an update is running"`, `"Claude Code not found"` |
| `claude_update_run` | — | `{ "started": true }` immediately; progress via the Setup topic | `"an update is running"`, `"Claude Code not found"` |

`claude_update_run` does not require `available`: `claude update` decides (it is the only way out when the check itself fails). A check is refused while an update runs; an update is refused while a check runs only until that check ends (the core may simply wait for it).

## 3. Status bar / banner inputs

No new event: the UI derives everything from `setup.claude` (the `update` object above). The Orchestrator updater's `update` event is unrelated and unchanged.
