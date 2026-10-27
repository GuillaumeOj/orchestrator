# orch-team

Teams of agents (SPEC §4.3, §4.4, §7): the data the canvas edits and the rules the orchestrator
enforces. Pure: no I/O, no clock. The store persists these types as JSON; the agent runtime
turns a compiled node into its `Role`.

| Module | What |
|---|---|
| `model` | `RoleDef` (FR-303), `Team` = `AgentNode`s + `Edge`s (`reports-to`, `hands-off-to`, `reviews`, `can-message`, FR-304), `Scope` (global / project), `PermissionProfile` (strict → `manual`, gated → `acceptEdits`, autonomous → `auto`, FR-706). Node ids are git-safe slugs: they name the worktrees `orch/<feature>/<node>-<n>`. |
| `validate` | Inline issues for the canvas (FR-305): no lead / several leads, unreachable node (no path to the lead over any edge), reports-to cycle, dangling / self / duplicate edges, unknown role, invalid or reserved node ids (`integration`, `solo`, `user`), concurrency 0, a lead above 1. |
| `rules` | `can_message`, `can_assign`, `can_handoff`, `can_request_review`, `reports_to`, and the Rules-panel text. `USER` is the user's pseudo-node (FR-409). |
| `compile` | `compile_node(team, node, roles, feature) -> NodeConfig`: prompt = role prompt + node override + team context (nodes, rules) + the **lead protocol** (FR-401) or the specialist's working rules; model/effort/permission mode with node overrides; denied Claude Code tools (`Agent` unless `subagents_allowed`, FR-410; `WebFetch`/`WebSearch` unless `web`…); the orchestrator tools the node may call. |
| `presets` | Starter roles (Lead, Backend, Frontend, UI, UX, Reviewer) and presets Solo dev, Full-stack feature team, Bugfix squad, Design sprint (FR-306). |
| `export` | `TeamFile { format, team, roles }` JSON import/export, validated on import (FR-307). |
| `run` | `RunState` (§6.2) with allowed moves, `TaskStatus` (the board columns), `blocked_by` (FR-404). |

## Orchestrator tools per node

| Who | Tools (`mcp__orch__…`) |
|---|---|
| everyone | `commit`, `send_message`, `inbox`, `list_tasks`, `handoff`, `request_review`, `ask_user` |
| lead | + `create_task`, `update_task`, `request_spec_approval`, `merge`, `request_pr` |
| specialists | + `update_task` (own tasks), `report_done`, `sync_integration` |
| solo session (P1) | `commit`, `request_pr` |

`request_spec_approval`, `merge` and `sync_integration` are not in FR-402: the sandbox can't write
the repository's git dir from a linked worktree (FR-702 known gap), so merges are done by the core,
like commits.

## MCP registry model (`mcp`, P3 FR-601/FR-1202)

`McpServer` (stdio, http, sse or a claude.ai `connector`; `env` values may be `keychain:<account>`),
`Registry` (global + one project's overrides; `get` = project override > global), `resolve` (a
role's `mcp_refs` → `mcpServers` entries; refuses an unknown ref or a Keychain env without the
entry's `allow_secret_env`; a connector is left out and turns strict MCP off) and `role_issues`
(for `teams_view`: errors, and the connector + strict MCP warning). `RoleDef.mcp_refs` defaults
to empty, so older roles and imported teams load unchanged; raw `mcp_servers` keep working.

## Starters, presets and design artifacts (P3, FR-306, FR-602)

- `presets::STARTER_VERSION` (2 = P3) and `role_fingerprint` / `team_fingerprint`: FNV-1a of the
  canonical JSON, without scope, version, preset flag or origin, and without default values, so
  fields added later don't change it. `V1_FINGERPRINTS` are the P2 starters'.
- v2 starters: UX and UI use the `penpot` registry entry and carry the design hand-off convention
  (`DESIGN_EXPORTS`: export frames under the feature's `design/`, commit, hand off the repo path
  with `kind="design"`); the lead orders frontend after the design task; Full-stack has
  `ui → front` hands-off-to (D2).
- `artifact::parse`: a hand-off URI → `ArtifactMeta` (Penpot, Figma, repo file, URL).

## Readings of the spec (simplest reading taken)

- **Messaging**: every node may message the lead and the lead everyone; reports-to and reviews
  allow messages both ways, hands-off-to one way (with the hand-off), can-message both ways.
- **Assigning**: the lead assigns to anyone; a node assigns to the nodes that report to it.
- **Reachability**: a node is reachable when some chain of edges (any type, any direction) links it
  to the lead.
- **Lead concurrency**: the lead owns the integration worktree, so it runs as one agent.

Tests: `cargo test -p orch-team` (unit tests per module, presets valid, round trips) and
`tests/properties.rs` (proptest: rules symmetric where they should be, validation never panics).
