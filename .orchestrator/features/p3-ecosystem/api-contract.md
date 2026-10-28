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
  env?: Record<string, string>;                // values may be "keychain:<account>" (Keychain account "mcp:<account>")
  allow_secret_env: boolean;                   // the FR-110-style opt-in; false → keychain refs are refused at compile
  connector_name?: string;                     // connector: the claude.ai connector's display name
  design_tool?: 'penpot' | 'figma' | null;
  status: Status; detail: string | null; checked_at_ms: number | null;
  used_by: { team: string, node: string, role: string }[];  // filled by the view
};
```

MCP env secrets (`keychain:<account>` values) live in their own Keychain namespace, account `mcp:<account>`, apart from the integrations' tokens (`linear`, `notion`):

| Command | Args | Returns |
|---|---|---|
| `mcp_secret_set` | `account: string, value: string` | `()` — stored under `mcp:<account>`; the value is never logged or returned. `account`: a-z, 0-9, `-_.`, ≤ 64; an empty value is refused |
| `mcp_secret_delete` | `account: string` | `()` (deleting a missing one is fine) |
| `mcp_secrets_status` | `accounts: string[]` | `Record<account, boolean>` (stored or not; an invalid name is `false`) |

Roles (P2 `role_save` / `teams_view`, extended, back-compatible):

```ts
// RoleDef additions
mcp_refs: string[];                            // registry ids; resolved at compile (project override > global)
downgrade: { to: string, when: 'amber' | 'red' | 'model_window' } | null;   // FR-819
// teams_view → roles[].issues may now include "mcp ref `x` not in the registry", "connector `x` with strict MCP"
```

Inbox (P1 shapes): a new alert kind payload `{"alert":"mcp_needs_auth","server":id,"name":…,"roles":[…]}` with actions `open` (→ Settings › MCP servers), `reconnect` (D3: http/sse → a pane running `claude /mcp`; connector → the claude.ai connector settings in the browser; stdio → Settings with the server selected) and `reject` (dismiss until the next failure). Connector + strict MCP is a **warning** issue: the compile turns strict MCP off for that role.

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
| `artifact_preview` | `feature: string, artifact: number` | `{ mime: string, data_b64: string, bytes: number, source: 'integration' \| 'branch' }` or error `"too large (<size> MB)"` (one decimal, MiB; cap 5 MB), `"not found on branch"`, `"not previewable"` |
| `artifact_open` | `feature, artifact` | `()` — links open in the browser, files in Quick Look (`qlmanage -p`) from the integration worktree |

Limits: 5 MB per preview; SVG is returned as data for an `<img src="data:image/svg+xml;base64,…">` only (never inlined into the DOM).

## 4. Presets (FR-306)

| Command | Args | Returns |
|---|---|---|
| `team_duplicate` | `team: string, from: Scope, to: Scope, newId?: string` | `{ team, roles_copied: string[] }` |
| `starter_copy` | `starter: string` (starter preset id), `to: Scope`, `newId?: string \| null` | `{ team, roles_copied: string[] }`: the starter as the app ships it (not the installed, maybe edited copy), id `newId` or `<starter>-new`, version 1, not a preset. A role visible in `to` that differs from the shipped one is copied in as `<role>-<team id>` and the nodes use it; a missing role is copied under its own id. |
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
  suggestions: string[];        // "What to change for next week": plain sentences (pp, never $); [] when nothing stands out
};
```
Inbox: `{"kind":"info","digest":"weekly_report","report":id}` (level 4); `open` → planner report view. `inbox_act(item, 'open')` on it marks it read and returns `{ open: 'report', report: id }`.
`reports_list` lists the current week first (`id: 'current'`, `in_progress: true`, `final_weekly_pct` = W now), then the stored weeks newest first. Rows (`by_*`) include an `outside` row ("outside the app") when the weekly reading counts more than the app's own samples; `share` is 0–1 of the week's total. Times (`week_start`, `reset_at`, `ts`) are RFC 3339.

## 6. Pacing additions (FR-814, FR-815, FR-816, FR-819)

`planner_view` (P1) gains:

```ts
snapshot.model_windows: { name: string, family: ModelFamily, used_pct: number, resets_at: string, state: 'green' | 'amber' | 'red' | 'exhausted' }[];   // [] when never seen
snapshot.calibration: { … P1 fields, per_model?: Record<ModelFamily, { k: number, samples: number }> };
off_hours: { allowance_pct: number, windows: { start: string, end: string }[] /* [] when allowance_pct is 0 */, live_cap: number, queued: number, running: number };
queue[].reason: … P1 reasons (incl. 'off_hours' = outside working hours, unchanged) | { kind: 'scheduled_off_hours', starts_at: string } /* D4 */ | { kind: 'model_window', model: ModelFamily, resets_at: string, exhausted: boolean };
actions[].kind: … | 'downgrade' | 'restore_model';
// a downgrade action: { kind: 'downgrade', agent: string, to: string, from: string | null, why: 'amber' | 'red' | 'model_window' }
// (plus the P1 action fields: reason, expected_effect, mode, outcome, undo…); restore_model: { kind: 'restore_model', agent }
```

`app_state` (P1) gains:

```ts
power: { on_ac: boolean | null, assertion_held: boolean, off_hours_pending: number };
banners: … | { kind: 'not_on_ac' };
agents[].model_note: string | null;           // "downgraded (Red)" while a downgrade is active
agents[].off_hours: boolean;                  // running work the user scheduled off-hours now
```

| Command | Args | Returns |
|---|---|---|
| `queue_off_hours` | `entry?: string, task?: string, off: boolean` | `{ starts_at: string }` — mark a queued entry or a task (its next turn) for off-hours, or clear it |
| `team_launch` (P2) | `form.off_hours?: boolean` | unchanged |
| `plan_set` (P1) | `form.off_hours_live_cap?: number`, `form.per_model_weights?: boolean` (FR-814; absent = unchanged) | unchanged |
| `plan_form` (P1) | — | the form, with `per_model_weights: boolean` (and `off_hours_live_cap`) from the pacing policy |
| `governor_act` (P1) | `action: 'undo'` on a `downgrade` action restores the model at the next task boundary | unchanged |

Notes (back, T-8):
- `queue_off_hours` takes `entry` / `task` as strings (`"12"`; a task's `"T-12"` too); numbers are accepted as well.
- Undoing a `downgrade` keeps the current task on the fallback model; the role's model comes back at the agent's next task (`restore_model` is not logged for an undo). In shadow mode a downgrade is logged once per episode.
- `power.off_hours_pending` counts real work: queued entries of marked agents plus scheduled turns running; a cancelled entry or a deleted agent drops its mark. The on-battery alert is once per night (an evening and the morning after are one night).
- `queue_off_hours` is refused with `"the plan has no off-hours allowance: set one in the Week planner first"` when the plan's `off_hours_allowance_pct` is 0 (the off-hours cap is active only with an allowance, spec §3.7). With `entry`, the mark lasts for that entry's turn; with `task`, for every turn of the task; clearing an entry also clears its agent's task mark. `starts_at` is RFC 3339 (now when already outside working time).
- While scheduled off-hours work runs and the off-hours cap is full, its reason is the P1 `{ kind: 'live_cap', state, cap }`.
- `queue[].reason` model window: `exhausted: false` = Red (≥ 95 % or the week's Red rule on that window; critical work still runs), `true` = used up (≥ 100 % or a model-specific rate limit).
- `team_launch` `form.off_hours` marks every task of the run.
- `app_state.banners` is a new array (only `{ kind: 'not_on_ac' }` from the core for now); `power.on_ac` is `null` until off-hours work first waits (the power source is read only then).
- `agents[].model_note` is `null` without a downgrade.
- `RoleDef.downgrade` round-trips through `role_save` / `teams_view` as `{ to, when }` (absent when off); `teams_view` roles get the warning issue `"model downgrade on a lead or reviewer role"`.
- `snapshot.model_windows[]` and `off_hours.windows[]` use RFC 3339 times (`resets_at`, `start`, `end`).

## 7. Task backend: Linear (FR-501, FR-503)

`project_settings` / `project_settings_set` (P2): `task_backend` accepts

```ts
{ type: 'linear', team: string /* key, e.g. "ENG" */, project?: string /* id */, states?: Record<string /* Linear state id */, 'backlog' | 'ready' | 'in_progress' | 'review' | 'done'> }
```

| Command | Args | Returns |
|---|---|---|
| `linear_teams` | — | `{ teams: { id, key, name }[] }` (needs the `linear` integration) |
| `linear_states` | `team: string` | `{ states: { id, name, type, default_column }[], projects: { id, name }[] }` |

D8: `project_settings_set` (P2 form, plus `migrate?: 'push_open' | 'leave'`) with a change **to** Linear on a project that has tasks needs `migrate`. Without it **nothing is saved** and the command *succeeds* with `{ confirm: { kind: 'linear_migrate', open: number, done: number } }` (no error text to parse); otherwise it returns the usual `project_settings` answer. `push_open` gives the open tasks to Linear (created at the sync it runs at once); `leave` keeps them local and unsynced (their `backend` stays `local`/`github`). Done tasks are never pushed. Any other `migrate` value is an error.

`board_sync` (P2) returns, for every backend, `{ pushed: number, pulled: number, imported: number, updated: number, conflicts: number, errors: string[] }` (`pulled` = imported + updated + conflicts). Conflicts (remote won) also create an Inbox info item (`{"action":"task_conflict","task":id,"project":id}`). A background sync runs every 5 min for projects with an active run.

`board_view` (P2) gains: `linear_team: string | null` (the team key), `last_sync_at_ms: number | null`, `last_sync: <the board_sync shape> | null`, `last_sync_error` (also from background syncs), and per task `remote_won: boolean` (the last sync kept the remote's version of it), `off_hours: boolean` (the task, or its run, is marked "Run off-hours"). For Linear, `tasks[].external_id` is the issue identifier (`"ENG-142"`), and `backend` is `'linear'` (a task left out by D8 keeps its old `backend`).

Back (T-9) notes: the transport is GraphQL at api.linear.app, core only; the API key is the Keychain's `linear` account, sent in a header on curl's stdin; the body goes through a 0600 temp file. A 429 or a `RATELIMITED` answer pauses that project's Linear calls (Retry-After, else 60 s doubling to 1 h); the sync then fails with `"Linear rate limit: retrying in N s"`. A Linear state added after the mapping was saved gets its column from its type; `"Linear state <id> isn't mapped: map it in the project settings"` only shows for an unknown state id.

## 8. Notion (FR-505, C)

| Command | Args | Returns |
|---|---|---|
| `notion_publish` | `feature: string` | `{ page_url: string, published_at_ms: number, docs: string[] }` |
| `notion_status` | `feature` | `{ page_url: string \| null, published_at_ms: number \| null, comments_seen: number, error: string \| null }` |

Project settings gain `notion_parent_page?: string` (a page URL or id; the page must be shared with the Notion integration). Notion comments → Inbox `{"kind":"question","source":"notion","page_url":…,"author":…,"text":…,"comment":id}` with action `open`.

Back (T-11) notes:
- `notion_publish` makes one page per feature (titled with the feature's title) under the parent page, with an intro paragraph and one child page per doc of `.orchestrator/features/<slug>/` on the integration branch (`spec.md` first). Publishing again refreshes the same pages (their blocks are replaced). Errors: `"set the project's Notion parent page in its settings first"`, `"feature <slug> has no docs on its integration branch"`, `"no Notion token: add it in Settings › Integrations"`, `"Notion: not found (…); share the page with the integration"`, `"Notion: not authorized (…)"`, `"Notion rate limit reached: try again later"`.
- Markdown → blocks: headings 1–3, paragraphs, bulleted/numbered lists, fenced code (with its language when Notion knows it), tables, inline links and `code`; anything else (HTML, quotes, rules) as a plain code block.
- Comments are read every 15 min while the feature has an active run, and on demand by `notion_status` (at most once a minute). The Inbox item is level 3, `kind: 'question'`, actions `["o", "r"]`: `open` (→ `inbox_act` returns `{ open: 'url', url: page_url }` and marks it read) and `reject` = dismiss. `author` is the commenter's name when Notion gives it, else their user id.
- `notion_status.error` is the last publish or poll error (`null` when it worked).

## 9. Settings pane

The Settings pane reads `settings_get`/`settings_set` (P1) for the existing keys and the views above for Integrations and MCP servers.

| Command | Args | Returns |
|---|---|---|
| `open_url` | `url: string` | `()`: opens an `http(s)://` link in the default browser; any other scheme (or a URL with whitespace / no host) is refused with `"only http and https links can be opened"`. Used for the connector reconnect (D3). |

Menu: **Orchestrator › Settings…** (`CmdOrCtrl+,`) emits the `menu` event with id `settings` (like the other menu items); the UI opens the Settings pane.
