# Update Claude Code from the app · Feature spec

> Feature `add-update-claude-code` · Integration branch `orch/add-update-claude-code/integration` · core (Rust) + Settings › Claude Code + status bar, with a short design pass

## 1. Goal

You see that a newer Claude Code exists (Claude Code's own "update available" notice in agent panes), but the app gives you no way to install it: agents run with `DISABLE_AUTOUPDATER=1` (FR-1201), and the only hint is "run `claude update`" in the read-only banner. This feature lets you **see** the installed and the latest Claude Code versions and **update** with one click, from **Settings › Claude Code** (today a "Not designed yet" placeholder) and from the status bar. No terminal and no app restart needed.

## 2. What exists today

- `agent-runtime/src/preflight.rs`: `claude_version()` (`claude --version`, cached per binary path + size + mtime), `SUPPORTED_CLAUDE` (min 2.1.302, tested max 2.1.302, not strict: newer runs as **untested**, with a warning).
- `orch-app/src/setup.rs`: `check_claude()` → `ClaudeCheck { path, version, untested, logged_in, auth_method, subscription, problem, note }`, run once at start off the startup path (`core.rs`, thread), stored in `core.claude`, served as `app_state.setup.claude`; topic `Setup` is touched when it lands.
- A `problem` (not found, too old, not logged in, not Pro/Max) makes the app read-only and shows the red "Read-only:" banner (`Banner.svelte`) with "Set up". A too-old version says "update Claude Code (claude update), then restart the app."
- `SetupPane.svelte` step 1 shows path · version (+ untested note).
- `SettingsPane.svelte` already lists a `claude` section ("Claude Code"), rendered as the "Not designed yet" placeholder.
- Installed here: native installer, `~/.local/bin/claude → ~/.local/share/claude/versions/2.1.317`. `claude update` exists ("Check for updates and install if available; package-manager installs update through the package manager"); it has no check-only mode.

## 3. How it works, end to end

```
start ── check_claude (as today) ──→ latest-version check (registry, ≤ 15 s) ──→ setup.claude.update
every 6 h / Settings "Check now" / palette ──→ latest-version check
installed < latest  → status bar chip "Claude Code 2.1.330", Settings nav badge, section offers "Update to 2.1.330"
Update ─→ `claude update` (core subprocess, user env, auto-update NOT disabled, ≤ 10 min, output kept)
       ─→ success: re-run check_claude (new version, untested?, login) → touch Setup → read-only banner clears if it was the version
       ─→ failure: error + the command's output in the section; nothing else changes
running agents keep the binary they started with; agents spawned or resumed afterwards use the new one
```

## 4. Scope

### 4.1 Core: latest version, update, re-check (back, T-1)

In `crates/orch-app/src/setup.rs` (or a new `claude_update.rs` beside it; owner's choice). `agent-runtime` changes only if a helper is needed (e.g. exposing `login_path`).

- **Install method** (display + hints only), from the resolved `claude` path (canonicalised): `native` (under `~/.local/share/claude/versions/`), `homebrew` (under a Homebrew prefix / Caskroom), `npm` (under a `node_modules/@anthropic-ai/claude-code`), else `unknown`.
- **Channel**: `autoUpdatesChannel` read from `~/.claude/settings.json` (`latest` | `stable`, default `latest`). Read only, never written (FR-1206).
- **Latest version**: `GET https://registry.npmjs.org/@anthropic-ai/claude-code` dist-tags (or the abbreviated `/-/package/@anthropic-ai/claude-code/dist-tags`), the tag named by the channel. Through `curl -fsS --max-time 15` (the app has no HTTP client; `update.rs` uses `gh`, which is not needed here). The owner checks that this matches what `claude update` installs for the native channel; if the native installer's release endpoint is the better source, use it and say so in the report.
  - When: once after the start-up `check_claude`, then every 6 h while the app runs, and on demand (`claude_update_check`). Never while an update runs. A failed check sets `update.error` and keeps the last known `latest`.
  - `available` = `latest` > installed version (`preflight::Version` comparison).
- **Update** (`claude_update_run`): runs `<claude path> update` as a core subprocess:
  - env: the user's login-shell PATH (`agent_runtime::env::login_path`), `HOME`, `USER`, `TERM=dumb`, `SHELL`; **without** `DISABLE_AUTOUPDATER` and without `ANTHROPIC_API_KEY`; stdin null; cwd `HOME`.
  - deadline 10 min (killed after), stdout + stderr merged, the last 40 lines kept in `update.log` (ANSI stripped), streamed: the Setup topic is touched at most every 500 ms while lines arrive.
  - one at a time: a second call while one runs is refused; refused when `claude` was not found.
  - on exit: `check_claude` runs again (new `ClaudeCheck`, replacing `core.claude`); `preflight::claude_version`'s cache already keys on size + mtime of the resolved binary: the owner checks it follows the symlink (a test with a swapped symlink). Then the latest check runs again.
  - outcome: exit 0 and version changed → `updated { from, to }`; exit 0 and version unchanged → `error` "claude update finished but Claude Code is still 2.1.317" + log (e.g. a package manager that pins it); non-zero / timeout → `error` with the last line + log.
  - nothing else is touched: no agent is stopped, suspended or restarted; spawns are not held.
- **Read-only state clears without a restart**: the re-check replaces `core.claude`; whatever reads `ready()` reads the new value. The too-old message becomes "… update Claude Code (Settings › Claude Code, or `claude update`)." (no "restart the app").
- **Payload**: `app_state.setup.claude` gains `problem_kind` (`not_found` | `version` | `login` | `plan`, set with `problem`), `install`, `channel`, `supported { min, tested_max }` and `update { … }` (api-contract §1). Commands in api-contract §2.
- **Tests** (Rust): install method from paths; channel parsing (missing file, missing key, bad JSON → `latest`); dist-tags parsing; `available` comparison; update outcomes with a fake `claude` script (`update` prints lines and swaps a version, exits 0; exits 0 without change; exits 1; never exits → timeout with a short test deadline); refusal while running; `DISABLE_AUTOUPDATER` absent from the child env.

### 4.2 Design pass (ux T-2, ui T-3)

[ux-brief.md](ux-brief.md): Settings › Claude Code section, the status bar chip, the read-only banner's new button, the Setup step 1 button, palette entries. ui draws the frames in FigCraft (ADR-012) and exports them to `design/` in this folder.

### 4.3 UI (front, T-4, after T-1 and T-3 are merged)

- New `ui/src/components/ClaudeSettings.svelte`, rendered by `SettingsPane.svelte` for section `claude`, replacing the placeholder. Nav badge on "Claude Code" when an update is available or `problem` is set.
- Pure helpers in `ui/src/lib/claude.js` (copy, states, chip), tested in `ui/test/claude.test.js` (vitest), in the style of `lib/update.js`.
- Status bar chip (`StatusBar.svelte`), beside the app updater's chip; click opens Settings › Claude Code.
- `Banner.svelte`: the "Read-only:" banner gets "Update Claude Code" when the problem is the version (`problem_kind == "version"`), next to "Set up".
- `SetupPane.svelte` step 1: same button in that case, and the update state inline.
- Palette: "Check for Claude Code updates", "Update Claude Code" (disabled with a hint when nothing is available or one runs).

## 5. Out of scope

- Claude Code's own notice inside agent panes (its TUI; we don't touch it).
- Picking a specific version, rolling back, or changing the channel from the app.
- Updating automatically (still the user's call, FR-1201).
- Restarting running agents onto the new version (they pick it up when they restart / resume).
- Raising `SUPPORTED_CLAUDE.tested_max` (the contract tests decide that, separately).

## 6. Acceptance

1. With a newer version published, Settings › Claude Code shows installed and latest, "Update to <latest>" works, and afterwards shows the new version with no app restart; the chip disappears.
2. With the installed version below `supported.min`, the read-only banner offers "Update Claude Code"; after it succeeds the app leaves read-only without a restart.
3. A failing `claude update` shows the error and its output; the installed version and agents are unchanged.
4. Running agents keep running during and after the update.
5. `make ci` passes (new Rust and vitest tests included).
