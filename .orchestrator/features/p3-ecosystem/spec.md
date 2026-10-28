# P3 — Ecosystem · Feature spec

> Feature `p3-ecosystem` · Integration branch `orch/p3-ecosystem/integration` · Source: [docs/SPEC.md](../../../docs/SPEC.md) §11 P3 (v0.10)
> Companions: [ux-brief.md](ux-brief.md) (flows and screens to design) · [api-contract.md](api-contract.md) (core ↔ UI)

## 1. Goal and exit criterion

SPEC §11: *"P3 — Ecosystem: Linear, Notion, Penpot/Figma design flow (4.6), presets, weekly report, estimator, per-model limits, off-hours scheduling, optional model downgrade."*
**Exit criterion: a design → code flow on a real feature.** A team with UX and UI roles that have the Penpot (or Figma) MCP produces frames, hands them off, the frames show on the Feature page, and Frontend implements from them, with no copy and paste of links by the user.

So the order of work follows the exit criterion: **the design flow first** (registry, auth status, artifacts, previews, presets), then the pacing items, then Linear, then Notion.

## 2. What P2 already ships (do not redo)

| Item | State in `main` (e981cfa) |
|---|---|
| Starter roles and presets (FR-306) | `orch-team::presets`: Solo dev, Full-stack, Bugfix squad, Design sprint; installed into the library; versioned on save. Missing: duplicate, save-as-preset, design MCP on UI/UX. |
| Role MCP servers (FR-601) | `RoleDef.mcp_servers` is a raw `mcpServers` map copied into `--mcp-config`. No registry, no UI to pick servers, no auth status. |
| Artifacts (FR-602) | `handoff(artifact, to, kind?, note?)` stores an `artifacts` row (`kind`, `uri`, `from_node`, `to_node`, `note`) and messages the recipient. The Feature pane lists them as text. No previews, no design kind semantics. |
| Estimator (FR-814) | Done in `governor` (`estimator.rs`, confidence marker in the title bar and planner). Missing (governor README "Not done yet"): the calibration is not carried across weeks by a restart; per-model estimator weights (deferred by ADR-007). |
| Off-hours | `WeeklyPlan.off_hours_allowance_pct` and the curve support an allowance; `LiveCaps.off_hours = 0`; `QueueReason::OffHours` exists. Missing: FR-816 scheduling, AC power, power assertion, banner. |
| Task backends | Local and GitHub Issues (`orch-tasks`), `TaskBackendConfig` = `local` / `github`. |
| Secrets | No Keychain code yet. NFR-3: secrets only in the macOS Keychain. |
| Settings | Key/value `settings_get`/`settings_set`; no Settings pane (§9.3 lists one). |

## 3. Scope

Priorities are the SPEC's. Everything **M** and **S** below is in scope; **C** items are in scope but last, and are the first cut if the week runs short.

### 3.1 Integrations, secrets and the MCP registry — FR-1202 (M), FR-1203 (S), FR-604 (M), NFR-3
- **Keychain** (`orch-secrets` module in the core, Security framework, service `Orchestrator`, account `<integration>`). Secrets are never written to the DB, logs, events, argv or any agent's environment or per-agent files. The UI sends a secret once (set) and never reads it back (only `has_secret: bool`).
- **Integrations**: GitHub (status from `gh auth status`, already used), Linear (API key), Notion (internal integration token), Penpot, Figma. Each has a status `connected | needs_auth | error | not_configured | unknown`, a checked-at time, a short detail, and **Reconnect / Test** actions.
- **MCP server registry** (global, with per-project overrides): named entries `{ id, name, kind: stdio | http | sse | connector, command/args/env or url, design_tool?: penpot | figma }`. `connector` = a claude.ai connector that Claude Code loads from the user's login (e.g. "claude.ai Penpot Perso"); it can't be put in `--mcp-config`, so a role that uses one must not use `--strict-mcp-config` and the inspector says so.
  - A role references registry entries **by id** (`RoleDef.mcp_refs: Vec<String>`); at compile time the refs are resolved (project override > global) into the `mcpServers` map. Raw `mcp_servers` keeps working (back-compat, imported teams).
  - **Env secrets in an MCP entry** (`env` values of the form `keychain:<account>`) are resolved into the per-agent `mcp.json` only after an explicit opt-in per entry, with the FR-110-style warning ("the agent can read this token and send it to the model"). Default: OAuth-based remote servers, whose tokens Claude Code keeps itself and the app never touches (FR-1206).
- **Auth status per server (FR-604)**: the core probes each registry server used by a role (on Settings open, on Test, at agent spawn, and every 30 min while used): `http/sse` → MCP `initialize`; 401/403 = `needs_auth`. `stdio` → spawn, `initialize` + `tools/list` with a 10 s deadline. `connector` → `unknown` until seen (see next point). At runtime, a failed `mcp__<server>__*` call whose error looks like auth (PostToolUseFailure) marks the server `needs_auth`. **`needs_auth` creates one Inbox item per server** (alert, level 2, deduplicated, closed when the server is connected again), never a silent failure. *Backend verifies the probe against the installed Claude Code (2.1.302) and records findings in the crate README, as P0 did.*

### 3.2 Design flow — FR-601 (M), FR-602 (S), FR-603 (C)
- **FR-601**: the role inspector has an **MCP servers** picker over the registry (checkboxes, auth status dot, "connector: needs user settings" hint). The **Design sprint** and **Full-stack** presets give UX and UI the `penpot` registry ref (a registry entry `penpot` is seeded, `kind: connector`, editable), and their prompts tell them to export frames (below).
- **Design artifact convention** (written into the UI/UX starter prompts and the handoff tool description):
  - A design artifact is `handoff(artifact=<uri>, to, kind="design", note?)` where `uri` is a Penpot/Figma frame link **or** a repo path to an exported image.
  - Exported frames are committed by the designer under `.orchestrator/features/<slug>/design/<name>.png` (or `.svg`), on the designer's branch, then reach the integration branch through the Lead's merge.
  - The core parses `uri`: `design.penpot.app/...` → `{tool: penpot, file_id, page_id?, frame_id?}`; `figma.com/design/<key>/...?node-id=` → `{tool: figma, file_key, node_id}`; a repo-relative image path → `{tool: file, path}`. Stored on the artifact as `meta` (new nullable JSON column, migration).
- **FR-602**: the Feature pane gets a **Design** section: design artifacts grouped by from → to node, each with tool icon, frame name/link, note, "Open" (browser for links, Quick Look/editor for files) and "Handed to Frontend · 14:20". The handoff message to Frontend includes the repo path of each exported image. Agents can't read other worktrees (ADR-009), so the convention is: the Frontend task `depends_on` the design task, the Lead merges the design branch first, and Frontend gets the files in its own worktree (branched from the integration branch, or `sync_integration`) and `Read`s them there. The Lead's starter prompt says so.
- **FR-603 (C)**: thumbnails/previews of exported image artifacts on the Feature page: the core serves the image bytes from the **integration branch** (`git show orch/<slug>/integration:<path>`, max 5 MB, png/svg/jpg/webp; svg sanitized: served as an `<img>` only, never inlined) or, before the merge, from the designer's branch. Link-only artifacts show a tool icon and no preview (no network fetch of Penpot/Figma from the core in P3).

### 3.3 Presets — FR-306 (M, remainder), FR-307 kept
- **Duplicate** a team or role (to global or a project scope, new id `<id>-copy`, version 1).
- **Save as preset**: a team marked `preset: true` shows in the presets row of the canvas and the Launch dialog. Starter presets stay read-only templates (opening one and editing it saves a project/global copy, never mutates the starter).
- **Starter preset updates**: `install_presets` adds missing ones (today); P3 adds `starter_version` so an installed, **unmodified** starter is upgraded in place when the app's starter changes (modified ones are left alone and get an "update available" hint).

### 3.4 Weekly report — FR-818 (S)
- At each weekly reset (and on demand for the current week), the core builds a **WeeklyReport** from usage samples, the plan, allocations, governor actions and team runs, **persisted** in a new `weekly_reports` table (samples expire at 90 days, reports are kept).
- Content: totals (weekly % at reset, agents' share, interactive, outside the app), **plan vs actual** per allocation (weight, planned pp, used pp, delta), usage per **project / feature / role / model** (pp of the week and $ estimate), time in each pressure state, governor actions by kind (count, with undone count), alerts, limit hits, queued time (p50/max) per allocation.
- Attribution: agent → project, team run → feature and node → role; model from OTEL `api_request` (fallback: the role's model). Unattributed usage is its own row ("outside the app").
- A **"Weekly report ready"** Inbox item (level 4 digest) opens the report; past reports are listed in the Week planner (J8 · 4 storyboard already in Penpot `06`).

### 3.5 Estimator — FR-814 (S, remainder)
- **Carry the calibration across restarts and weeks**: persist `Calibration` rows (SPEC §7) on every fit change (throttled to 1/min) and restore the last one at start and at `new_week`; the replay of the week's samples continues from it. Test: restart mid-week and across a reset gives the same `k`, `u` and confidence as the live run (pacing-sim).
- **Per-model weights (ADR-007 deferred item)**: the cost → pp scalar `k` is fitted per model family (opus / sonnet / haiku / fable) when there are ≥ 20 samples of that family in the window, else the shared `k`. Off by default behind `PacingPolicy.per_model_weights`; NFR-9 must still pass with it on.

### 3.6 Per-model weekly limits — FR-815 (S)
- The statusline P0 payload had **no per-model weekly field** (spikes/p0-claude-cli.md). So the collector accepts any extra `rate_limits.<name>` object with `{used_percentage, resets_at}` beyond `five_hour`/`seven_day` (e.g. `seven_day_opus`) as an extra window `Window::Model(<name>)`, mapped to a model family by name. StopFailure `rate_limit` with a model-specific message also marks that window exhausted.
- Each model window has its own state (same thresholds as the week, computed on its own `used` vs the elapsed curve). When a model window is **Red or exhausted**, turns of agents whose current model is in that family are **queued** with reason `ModelWindow { model }`, unless the role opted into the downgrade (3.8), which then switches model at the task boundary. The planner shows one extra row per model window seen.
- Tested with synthetic statusline payloads; nothing is shown when no such window is ever seen.

### 3.7 Off-hours scheduling — FR-816 (S), J10.3
- **"Run off-hours"** on a queued entry, a task (Board/Feature), or at launch: the work is queued with `not_before = next off-hours window start` and tier kept; the Governor admits it only in off-hours, under `LiveCaps.off_hours` (new setting, default 1, active only when `off_hours_allowance_pct > 0`) and inside both plan windows.
- **Power**: while off-hours work is queued or running, the core holds a macOS power assertion that prevents idle system sleep **on AC only** (`IOPMAssertionCreateWithName(kIOPMAssertPreventUserIdleSystemSleep)` or `caffeinate -s` child tied to the core's pid). On battery: the global banner "Not on AC power: off-hours work won't run while the Mac sleeps" (§9.3 banner already listed) and an Inbox alert once per night. Released when the queue has no off-hours work.
- The planner's curve shows the off-hours windows and the queued off-hours work.

### 3.8 Optional model downgrade — FR-819 (C)
- Per role: `downgrade: { to: <model>, when: amber | red | model_window }` (off by default; never set on Lead or Reviewer starters; the inspector warns when set on them).
- Applied only at a **task boundary** (the agent is idle and its next turn is a new task): the core suspends and resumes the session with `--model <to>` (resume keeps the conversation; the prompt cache is lost, which is why it's opt-in). Reverted at the next task boundary once the condition clears. Logged as a governor action (undoable), shown in the agent header ("sonnet · downgraded (Red)").

### 3.9 Linear task backend — FR-501 (S), FR-503 (S), ADR-004
- `TaskBackendConfig::Linear { team: <key>, project?: <id> }`, API key from the Keychain (`linear`), GraphQL at `https://api.linear.app/graphql`, from the core only (agents never call it).
- Mapping (mirrors the GitHub one): label `orch` marks managed issues; columns map to the team's workflow states by type (`backlog`→Backlog, `unstarted`→Ready, `started`→In progress, a state named "In Review" or the first `started` state after it → Review, `completed`/`canceled`→Done), configurable per project; `orch:node:<id>` label for the assignee node; dependencies as Linear **issue relations** `blocks`; notes as comments.
- Two-way sync, **remote wins**, the user is notified (Inbox info), same `SyncReport` as GitHub, same pending-push retry. Rate limits respected (Linear's complexity budget: back off on 429/`RATELIMITED`).
- Tests with a fake GraphQL transport (like `Gh` for GitHub).

### 3.10 Notion publish — FR-505 (C)
- Per project: a Notion parent page id; per feature: **Publish docs** creates (then refreshes) one Notion page per feature with `spec.md`, `ux-brief.md`, `api-contract.md`, `decisions.md` as toggles or child pages, from the **integration branch** copy (FR-504). Markdown → Notion blocks: headings, paragraphs, lists, code, tables, links; anything else as a code block.
- Comments on the published pages are polled (every 15 min while the feature is active, and on demand) and come back as Inbox items (question kind, level 3) with a link; replying in the app is out of scope (open in Notion).
- Token from the Keychain (`notion`), core only.

## 4. Out of scope (P3)
- Fetching Penpot/Figma images from their APIs in the core (only exported files in the repo are previewed).
- Notion → repo sync of edits; replying to Notion comments from the app.
- Linear webhooks (polling only, like GitHub).
- A real per-model limit hit (not reproducible on demand; tested with synthetic payloads, R10 stays open).
- Changes to the SPEC text itself: findings go to `decisions.md` here, and the SPEC is updated in a follow-up, as v0.10 did for P2.

## 5. Non-functional
- **Security (NFR-3, FR-703, ADR-009)**: tokens only in the Keychain; never in agent env or per-agent files except the explicit MCP-env opt-in; the core is the only network client of Linear/Notion. Test: a team agent can't read the Linear/Notion token (no env, no file, `security find-generic-password` denied by the sandbox).
- **Memory (NFR-1b)**: no new long-lived processes except the power assertion; MCP probes are short-lived with deadlines; previews are lazy and capped (5 MB, thumbnails ≤ 480 px rendered by the webview with `loading="lazy"`).
- **Data (NFR-8)**: migration 3 adds `artifacts.meta`, `weekly_reports`, `calibrations`, `mcp_registry`, `integrations` (status only, no secrets); retention unchanged; reports and calibrations are kept.
- **Pacing quality (NFR-9)**: pacing-sim stays green with per-model weights on and off, off-hours work in the synthetic weeks, and model windows.
- **Tests**: `make ci` green (fmt, clippy -D warnings, cargo test, biome, svelte-check, vitest, ui build).

## 6. Acceptance (demo at the PR gate)
1. Settings → Integrations shows GitHub connected, Linear and Notion configurable with a Keychain-stored token, Penpot/Figma registry entries with status; a revoked token shows `needs_auth` and one Inbox item.
2. **Design → code (exit criterion)**: launch the Design sprint (or Full-stack) preset on a real small feature of this repo. UX writes the brief, UI produces frames in Penpot through the MCP, exports them to `.orchestrator/features/<slug>/design/`, hands off to Frontend; after the Lead's merge the Feature page shows the previews; Frontend implements from them. No link copied by the user.
3. Duplicate a preset into a project and launch it.
4. Board on a Linear-backed project: create/move a task in the app → appears in Linear; change it in Linear → comes back (remote wins, notified).
5. Week planner: the current week's report and the last reset's report; per model, per role, plan vs actual.
6. Restart the app mid-week: same calibration and forecast as before (FR-814).
7. Synthetic model window at 95 % in the simulator/dev tools → opus turns queue with "model window", a role with downgrade switches to sonnet at its next task.
8. Queue a task "Run off-hours": it waits with its window, the power assertion is held on AC (`pmset -g assertions`), the banner shows on battery.
9. (C) Publish the feature's docs to Notion; a comment comes back to the Inbox.
