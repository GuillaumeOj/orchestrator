# orch-store

SQLite persistence for the Rust core ([ADR-005](../../docs/adr/005-local-persistence.md), SPEC §7,
NFR-2, NFR-8): one database in app data (`<home>/state/orch.db`), WAL, **one writer thread**,
versioned migrations, append-only events with retention, an audit log, and an FTS5 index over
Claude Code transcripts (the JSONL files stay the source of truth).

## Handle

```rust,ignore
let db = orch_store::Store::open(&home.join("state/orch.db"))?; // creates, migrates, WAL
let p = db.project("demo")?;                                    // reads: a separate connection
db.write(|tx| { tx.execute("…", [])?; Ok(()) })?;               // one transaction on the writer thread
db.write_detached(|tx| …);                                      // same, without waiting (events, samples)
db.flush()?;                                                    // everything queued so far is committed
```

- Writes are applied in the order they are queued. `write_begin` queues a write and returns a
  `Pending` to wait on (the runtime waits after releasing its own lock); consecutive detached
  writes share one transaction (one WAL commit per batch, each in its own savepoint).
- `tag`/`untag`: a unit enum's serde name and back (used for key columns).
- `Store` is `Clone + Send + Sync`; every clone talks to the same writer thread. Dropping the last
  clone drains the queue and joins the thread.
- `synchronous=NORMAL` in WAL: a crash of the process (kill -9) never loses a committed transaction
  (`tests/crash.rs`); a power loss can lose the last ones.
- Migrations: `PRAGMA user_version` = number applied. Append-only list in `migrations.rs`; a database
  newer than the build is refused (`SCHEMA_VERSION`).
- Payloads over 64 KB in events go to files in `orch.blobs/` next to the database (NFR-8).

## Tables and API

| §7 entity | Table | API (on `Store`) |
|---|---|---|
| Project | `projects` | `upsert_project`, `project`, `project_by_path`, `projects(with_archived)`, `touch_project`, `record_fetch`, `delete_project` |
| Worktree | `worktrees` | runtime-owned (below); `worktree(path)`, `worktrees_of(repo)`, `upsert_worktree_tx(tx, row)` |
| Agent, Turn, (holds) | `agents`, `turns`, `holds` | runtime-owned: `load_runtime`, `sync_runtime(RuntimeDiff)`, `runtime_is_empty` |
| InboxItem | `inbox_items` | runtime-owned; `inbox_item(id)`, `inbox_pending()` (read only); `decided_by` (FR-709) |
| one-shot allow | `one_shot_allows` | `add_one_shot`, `take_one_shot` (FR-705: one exact call of one agent) |
| PermissionRule | `permission_rules` | `add_rule`, `revoke_rule` (row kept), `rules()` (active) |
| UsageSample | `usage_samples` | `add_usage` (detached), `usage_since(ms, source)`, `usage_totals(ms)` |
| WeeklyPlan, BudgetAllocation | `weekly_plans`, `budget_allocations` | `save_plan(plan, PlanStatus)`, `active_plan`, `previous_plan(before)` |
| PacingPolicy | `pacing_policies` | `save_policy(id, p)`, `policy(id)` |
| QueueEntry | `queue_entries` | `replace_queue(boot, entries)`, `queue_entries` (a view; see below) |
| GovernorAction | `governor_actions` | `save_actions(boot, plan_id, actions)` (upsert by boot + id), `actions(limit)` |
| Calibration | `calibrations` | `add_calibration`, `latest_calibration` |
| ForecastSnapshot | `forecast_snapshots` | `add_forecast`, `forecasts_since(ms)` |
| Terminal | `terminals` | `upsert_terminal`, `delete_terminal`, `terminals` |
| (events) | `events` | `add_event(ts, agent, kind, detail)` (detached), `events(since, agent, kind_prefix, limit)` |
| (audit, FR-709) | `audit_log` | `audit(actor, agent, action, target, decision, detail)` (waits), `audit_log(agent, limit)` |
| (settings) | `kv` | `set_setting`, `setting` |
| Role (P2) | `roles` | `upsert_role(role, at)`, `role(scope, id)`, `roles(project)` (global + that project's, which shadow), `archive_role(scope, id)` |
| Team, AgentNode, Edge (P2) | `teams`, `team_versions` | `save_team(team, roles, at)` (also snapshots the version with the roles it uses, FR-308), `team(scope, id)`, `team_in(project, id)` (its own, else the global one), `teams(project)`, `team_version(scope, id, v)`, `archive_team(scope, id)` |
| Feature (P2) | `features` | `upsert_feature`, `feature`, `feature_by_slug(project, slug)`, `features(project)` |
| TeamRun (P2) | `team_runs` | `upsert_run`, `run`, `runs(feature, active_only)` |
| Task (P2) | `tasks` | `upsert_task` (id 0 = insert; returns the id), `task`, `task_by_external(project, backend, ext)`, `tasks(project, feature, run)`. Every backend is mirrored here (ADR-004). |
| Message (P2) | `messages` | `add_message`, `messages(run, Some((node, agent)), unread)`, `mark_messages(ids, agent, read, at)` |
| Artifact (P2) | `artifacts` | `add_artifact`, `artifacts(feature)` |
| (team timeline, FR-408) | `events.team_run_id` | `add_run_event(ts, run, agent, kind, detail)` (detached), `run_events(run, limit)` |
| (project settings, FR-109) | `projects.settings`, `task_backend` | `Project::typed_settings()`, `Project::backend()`, `set_project_settings(id, settings, backend, track)` |
| (search, FR-209) | `transcript_files`, `transcript_fts` | `index_transcripts(&[TranscriptFile])`, `search_transcripts(q, project, limit)` |

Governor entities are stored as the `governor` crate's serde types (JSON in `data`, key columns
for queries). Row types defined here: `Project`, `Remote`, `WorktreeRow`/`WorktreeRowState`,
`InboxItem`/`InboxStatus`, `PermissionRule`/`RuleScope`/`RuleEffect`, `UsageRecord`/`UsageSource`,
`Terminal`, `PlanStatus`, `Event`, `AuditEntry`, `Retention`, `SearchHit`.

### Runtime-owned tables

`agents`, `worktrees`, `turns`, `holds` and `inbox_items` are kept **in memory by the agent
runtime** for its hot paths (hooks, the 200 ms delivery pump) and written through on every change
with `sync_runtime(RuntimeDiff)`: one transaction per change, upserts and deletes. Read them freely
(SQL, `inbox_pending`, `worktrees_of`); **change them only through the runtime**
(`agent_runtime::runtime::Runtime`), or the runtime's copy goes stale.

### Retention

`purge(now, Retention)`: events 30 days, forecast snapshots 30 days, usage samples 90 days, decided
Inbox items 90 days (the runtime keeps the last 200 finished turns/holds/items in memory and
deletes older rows). The audit log is never purged.

### Transcript index

`index_transcripts` reads each file from its last indexed byte in 2 MB chunks, one transaction
per chunk, until caught up (complete lines only; a line over 16 MB is skipped), indexes user text, assistant text and tool calls (`tool: Bash cargo test`), skips tool
results and meta entries, and re-indexes a file that shrank. `search_transcripts` quotes every term
(all must match; `-`, `:` and `*` are never operators), ranks by BM25 and returns a snippet with
`[`…`]` around matches, the entry's `uuid` and the file path. `encode_project_dir(path)` gives
Claude Code's folder name under `~/.claude/projects`.

## Readings

- **Runtime rows as JSON + key columns**: `agents.data`, `turns.data` and `holds.data` hold the
  runtime's full records (its types live in `agent-runtime`); the key columns are for queries.
- **Holds** are not a §7 entity but the reconciler needs them (J10 step 7), so they have a table.
- **Queue entries** are a view of the Governor's queue for the UI. What a restart restores is the
  carry-over state in `kv` (`governor.carry`: the queue with its ids, pauses, deferrals, the ask,
  sprints, limits), written by the runtime.
- **Governor actions** restart their ids at each core start, so rows are keyed by (boot, id).
- **Roles and teams as JSON**: `roles.data` and `teams.data` hold `orch_team::RoleDef` / `Team`;
  `team_versions` keeps every version a run may point at, with the roles it used, and outlives an
  archived team.
- **Scoped ids** (migration 3): roles, teams and snapshots are keyed by (scope, id), `scope` being
  `''` for global or a project id. A project's role or team with a global one's id shadows it in
  that project and leaves it untouched elsewhere; a run records its team's scope (`team_scope`).
  A global team launched from a project that shadows one of its roles snapshots that project's
  role, so its version goes up when launched from projects whose roles differ.
- **MCP registry and integrations** (migration 5): `mcp_registry` rows are keyed by (scope, id)
  like roles (a project row overrides the global one in that project), `data` is the
  `orch_team::mcp::McpServer` definition and the last probe sits in `status`/`detail`/
  `checked_at_ms` (a changed definition resets it to `unknown`). The migration seeds the `penpot`
  connector entry once (deleting it sticks). `integrations` holds a status only: secrets live in
  the Keychain (NFR-3).
- **Artifact meta** (migration 6): `artifacts.meta` is the parsed hand-off URI
  (`orch_team::artifact::ArtifactMeta`, JSON), NULL for rows from before P3.
- **Weekly reports and estimator state** (migration 7): `weekly_reports` (FR-818, never purged);
  `calibrations` gains `kind` (`fit` or `week`), `week_start_ms` and the estimator `state` (FR-814).
- **Per-model windows** (migration 8): `usage_samples.model_windows`, JSON `[{name, used_pct,
  resets_at}]` of a statusline response's extra `rate_limits.<name>` windows (FR-815), NULL when none.
- **Tasks**: one model over every backend (ADR-004); `depends_on` are local task ids. `delete_project`
  removes the project's features, runs, tasks, messages, artifacts and project-scoped roles and
  teams; events stay.
- **Usage samples** carry the Governor's cost class and allocation at record time, so the week's
  accounting can be replayed after a restart (`governor::Governor::replay`).

## Tests

`cargo test -p orch-store`: writer/reader and rollback, concurrent writers, migrations (FTS5
present, newer schema refused), every entity, events with blobs and retention, the audit log,
incremental transcript indexing (partial lines, rewrites) and search, and `tests/crash.rs`
(a writer process killed with SIGKILL mid-stream: integrity check, no lost commit, atomic diffs).
