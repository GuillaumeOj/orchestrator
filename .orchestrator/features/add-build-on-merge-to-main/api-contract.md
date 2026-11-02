# Build On Merge To Main · API contract

Between the core (`crates/orch-app/src/update.rs`, T-2) and the UI (T-3). The CI side (T-1) is in spec §3.2.

## 1. Release assets (T-1 → T-2)

Release tag `v0.1.<n>` on `GuillaumeOj/orchestrator`, marked latest, with exactly:

| Asset | What |
|---|---|
| `Orchestrator_0.1.<n>_aarch64.app.tar.gz` | `Orchestrator.app` at the archive root |
| `Orchestrator_0.1.<n>_aarch64.app.tar.gz.sig` | minisign signature of the tarball (`tauri signer sign` output, base64 as Tauri writes it) |

The version is the tag without `v`. The release's `targetCommitish` is the full commit SHA.

## 2. Build info (compile time, T-1)

Env read with `option_env!` in `orch-app` (defaults in parentheses): `ORCH_BUILD_SHA` (`""`), `ORCH_BUILD_CHANNEL` (`"local"`). The version is Tauri's `package_info().version` (`0.1.0` for a local build, `0.1.<n>` from CI).

## 3. State in `app_state` and the `update` event

`app_state` gains `"update"`, and the core emits the Tauri event `update` with the same object on every change of `state` (the UI replaces its copy; no `refresh()` needed).

```json
{
  "current": { "version": "0.1.41", "sha": "bd5797c…", "channel": "main" },
  "enabled": true,
  "disabled_why": null,
  "state": "ready",
  "available": { "version": "0.1.42", "sha": "f9b1e20…", "published_at": 1793600000, "subject": "Merge pull request #23 …" },
  "progress": null,
  "error": null,
  "checked_at": 1793600123,
  "apply_on_quit": true
}
```

| Field | Values |
|---|---|
| `current.channel` | `"main"` (CI build) or `"local"` |
| `enabled` / `disabled_why` | `false` with a reason: `"debug build"`, `"ORCH_NO_UPDATE"`, `"bundle not writable"`, `"build in target/"`, `"no update key"` (the committed `update.pub` is still the placeholder), `"gh not installed"`, `"gh not logged in"` (the last two are rechecked at each check) |
| `state` | `"idle"` (up to date or never checked), `"checking"`, `"downloading"`, `"ready"` (verified and staged), `"error"` |
| `available` | the newer release, or `null` when none |
| `progress` | `0.0..1.0` while `downloading` when known, else `null` |
| `error` | short text when `state = "error"` (`"signature does not match"`, `"offline"`, `"gh: …"`), else `null`. The next successful check clears it |
| `checked_at` | Unix seconds of the last finished check, or `null` |
| `apply_on_quit` | `true` when `ready` and `channel = "main"`: quitting installs it |

Times are Unix **seconds**, like `usage.five_hour.resets_at`.

## 4. Commands

| Command | Args | Returns | Effect |
|---|---|---|---|
| `update_check` | none | the state object (§3) after the check starts | Check now (and download if newer). Refused (`Err`) when `enabled = false`. A check already running is not doubled. |
| `update_restart` | none | never returns on success | Only when `state = "ready"` (else `Err("no update ready")`): quit as ⌘Q does (agents suspended), swap the bundle, relaunch it. On a swap failure the old bundle is put back, the app stays up and the state becomes `error`. |

## 5. Menu / palette

The palette entries are UI-side (T-3) and call the commands above: "Check for updates" (enabled when `enabled`), "Restart to update" (enabled when `state = "ready"`), "About Orchestrator" (shows `current`).
