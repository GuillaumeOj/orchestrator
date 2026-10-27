# P3 — Ecosystem · API contract (core ↔ UI)

> Owners: `back` implements the Tauri commands in `crates/orch-app` (and what they call in `agent-runtime`, `orch-store`, `orch-tasks`, `orch-team`, `governor`); `front` calls them from `ui/src`. Spec: [spec.md](spec.md).
> Conventions (same as P1/P2): Tauri commands in `crates/orch-app/src/commands.rs`, errors are strings, **JS argument names are camelCase** (Tauri converts `with_prs` → `withPrs`), **response fields are snake_case** (serde). Times are epoch **ms** (`*_ms`) unless named `*_at` (RFC 3339). The UI polls views as today (no new event channel); `app_state` carries what the shell needs on every poll.
> A change to this contract goes through `lead` (message), who updates this file on the integration branch.

## 0. Shared types

```ts
type Status = 'connected' | 'needs_auth' | 'error' | 'not_configured' | 'checking' | 'unknown';
type ModelFamily = 'opus' | 'sonnet' | 'haiku' | 'fable' | string;
type Scope = { kind: 'global' } | { kind: 'project', id: string };   // as P2
```

## 1. Integrations and secrets (FR-1203, NFR-3)

| Command | Args | Returns |
|---|---|---|
| `integrations_view` | — | `{ items: Integration[] }` |
| `integration_secret_set` | `id: 'linear'\|'notion'\|string, secret: string` | `Integration` (status re-tested) |
| `integration_secret_remove` | `id` | `Integration` |
| `integration_test` | `id` | `Integration` |

```ts
type Integration = {
  id: 'github' | 'linear' | 'notion' | 'penpot' | 'figma';
  name: string;
  status: Status;
  detail: string | null;          // "logged in as GuillaumeOj", "401 from api.linear.app", …
  has_secret: boolean;            // never the secret itself
  needs_secret: boolean;          // linear, notion: true; github: false (gh); penpot/figma: false (MCP)
  checked_at_ms: number | null;
  account: string | null;         // workspace / user name when known
};
```
The secret is sent once in `integration_secret_set` and never returned. The core never logs the args of that command.

## 2. MCP registry and auth status (FR-1202, FR-604, FR-601)

| Command | Args | Returns |
|---|---|---|
| `mcp_registry_view` | `project?: string` | `{ servers: McpServer[], project_overrides: Record<string, McpServer> }` |
| `mcp_server_save` | `server: McpServer, scope: Scope` | `McpServer` (validated; error string on invalid) |
| `mcp_server_delete` | `id, scope` | `{ used_by: string[] }` (refused with the list if a role uses it) |
| `mcp_server_test` | `id, project?` | `McpServer` (fresh `status`) |

```ts
type McpServer = {
  id: string;                     // [a-z0-9-]+, the key in mcpServers and in RoleDef.mcp_refs
  name: string;
  kind: 'stdio' | 'http' | 'sse' | 'connector';
  command?: string; args?: string[];           // stdio
  url?: string;                                // http / sse
  env?: Record<string, string>;                // values may be "keychain:<account>"
  allow_secret_env: boolean;                   // the FR-110-style opt-in; false → keychain refs are refused at compile
  connector_name?: string;                     // connector: the claude.ai connector's display name
  design_tool?: 'penpot' | 'figma' | null;
  status: Status; detail: string | null; checked_at_ms: number | null;
  used_by: { team: string, node: string, role: string }[];  // filled by the view
};
```

Roles (P2 `role_save` / `teams_view`, extended, back-compatible):

```ts
// RoleDef additions
mcp_refs: string[];                            // registry ids; resolved at compile (project override > global)
downgrade: { to: string, when: 'amber' | 'red' | 'model_window' } | null;   // FR-819
// teams_view → roles[].issues may now include "mcp ref `x` not in the registry", "connector `x` with strict MCP"
```

Inbox (P1 shapes): a new alert kind payload `{"alert":"mcp_needs_auth","server":id,"name":…,"roles":[…]}` with actions `open` (→ Settings › MCP servers) and `reject` (dismiss until the next failure).

## 3. Design artifacts (FR-602, FR-603)

`feature_view` (P2) — `artifacts[]` gains `meta`:

```ts
type Artifact = {
  id: number; feature_id: string; team_run_id: string | null;
  kind: 'doc' | 'design' | 'build' | string; uri: string;
  from_node: string; to_node: string | null; note: string; created_at_ms: number;
  meta: null | {
    tool: 'penpot' | 'figma' | 'file' | 'url';
    title?: string;                              // frame/file name when the URI says it, else the file name
    file_id?: string; page_id?: string; frame_id?: string;   // penpot
    file_key?: string; node_id?: string;                     // figma
    path?: string;                                           // file: repo-relative
    previewable: boolean;                                    // file with png/jpg/webp/svg
    on_integration: boolean | null;                          // file: present on the integration branch now
  };
};
```

| Command | Args | Returns |
|---|---|---|
| `artifact_preview` | `feature: string, artifact: number` | `{ mime: string, data_b64: string, bytes: number, source: 'integration' \| 'branch' }` or error `"too large"`, `"not found on branch"`, `"not previewable"` |
| `artifact_open` | `feature, artifact` | `()` — links open in the browser, files in Quick Look (`qlmanage -p`) from the integration worktree |

Limits: 5 MB per preview; SVG is returned as data for an `<img src="data:image/svg+xml;base64,…">` only (never inlined into the DOM).

## 4. Presets (FR-306)

| Command | Args | Returns |
|---|---|---|
| `team_duplicate` | `team: string, from: Scope, to: Scope, newId?: string` | `{ team, roles_copied: string[] }` |
| `role_duplicate` | `role: string, from: Scope, to: Scope, newId?: string` | `RoleDef` |
| `team_set_preset` | `team: string, scope: Scope, preset: boolean` | `Team` |
| `preset_update` | `team: string, scope: Scope` | `Team` (re-copies an unmodified starter; refused if modified) |

`teams_view` teams gain `preset: boolean`, `starter: string | null` (the starter id it came from), `update_available: boolean`.

## 5. Weekly report (FR-818)

| Command | Args | Returns |
|---|---|---|
| `reports_list` | — | `{ reports: { id: string, week_start: string, reset_at: string, final_weekly_pct: number \| null, in_progress: boolean }[] }` |
| `report_view` | `id: string \| 'current'` | `WeeklyReport` |

```ts
type Row = { key: string, label: string, pp: number, cost_usd: number, share: number };
type WeeklyReport = {
  id: string; week_start: string; reset_at: string; in_progress: boolean;
  provenance: 'observed' | 'estimated';
  totals: { weekly_pct: number, agents_pp: number, interactive_pp: number, outside_pp: number, reserve_pct: number };
  allocations: { id: string, target: string, weight: number, tier: string, planned_pp: number, used_pp: number, delta_pp: number, queued_p50_min: number | null, queued_max_min: number | null }[];
  by_project: Row[]; by_feature: Row[]; by_role: Row[]; by_model: Row[];
  states: { state: 'green' | 'amber' | 'red' | 'catch_up', minutes: number }[];
  actions: { kind: string, count: number, undone: number }[];
  alerts: { ts: string, text: string }[];
  limit_hits: { window: string, ts: string, resets_at: string }[];
  model_windows: { name: string, family: ModelFamily, peak_pct: number }[];
};
```
Inbox: `{"kind":"info","digest":"weekly_report","report":id}` (level 4); `open` → planner report view.

## 6. Pacing additions (FR-814, FR-815, FR-816, FR-819)

`planner_view` (P1) gains:

```ts
snapshot.model_windows: { name: string, family: ModelFamily, used_pct: number, resets_at: string, state: 'green' | 'amber' | 'red' | 'exhausted' }[];   // [] when never seen
snapshot.calibration: { … P1 fields, per_model?: Record<ModelFamily, { k: number, samples: number }> };
off_hours: { windows: { start: string, end: string }[], live_cap: number, queued: number, running: number };
queue[].reason: … P1 reasons | { kind: 'off_hours', starts_at: string } | { kind: 'model_window', model: ModelFamily, resets_at: string };
actions[].kind: … | 'downgrade' | 'restore_model';
```

`app_state` (P1) gains:

```ts
power: { on_ac: boolean | null, assertion_held: boolean, off_hours_pending: number };
banners: … | { kind: 'not_on_ac' };
agents[].model_note: string | null;           // "downgraded (Red)" while a downgrade is active
```

| Command | Args | Returns |
|---|---|---|
| `queue_off_hours` | `entry?: string, task?: string, off: boolean` | `{ starts_at: string }` — mark a queued entry or a task (its next turn) for off-hours, or clear it |
| `team_launch` (P2) | `form.off_hours?: boolean` | unchanged |
| `plan_set` (P1) | `form.off_hours_live_cap?: number` | unchanged |
| `governor_act` (P1) | `action: 'undo'` on a `downgrade` action restores the model at the next task boundary | unchanged |

## 7. Task backend: Linear (FR-501, FR-503)

`project_settings` / `project_settings_set` (P2): `task_backend` accepts

```ts
{ type: 'linear', team: string /* key, e.g. "ENG" */, project?: string /* id */, states?: Record<string /* Linear state id */, 'backlog' | 'ready' | 'in_progress' | 'review' | 'done'> }
```

| Command | Args | Returns |
|---|---|---|
| `linear_teams` | — | `{ teams: { id, key, name }[] }` (needs the `linear` integration) |
| `linear_states` | `team: string` | `{ states: { id, name, type, default_column }[], projects: { id, name }[] }` |

`board_sync` (P2) returns the same `SyncReport` for every backend; conflicts (remote won) also create an Inbox info item.

## 8. Notion (FR-505, C)

| Command | Args | Returns |
|---|---|---|
| `notion_publish` | `feature: string` | `{ page_url: string, published_at_ms: number, docs: string[] }` |
| `notion_status` | `feature` | `{ page_url: string \| null, published_at_ms: number \| null, comments_seen: number, error: string \| null }` |

Project settings gain `notion_parent_page?: string`. Notion comments → Inbox `{"kind":"question","source":"notion","page_url":…,"author":…,"text":…}` with action `open`.

## 9. Settings pane

The Settings pane reads `settings_get`/`settings_set` (P1) for the existing keys and the views above for Integrations and MCP servers. No other new command.
