# Fix FigCraft status

## Problem

Settings › Integrations (and Settings › MCP servers) show **FigCraft: Unknown**, while the
sessions use FigCraft's tools (`mcp__figcraft__*`) without trouble.

## Why (from the code)

The FigCraft integration's status is the worst status of the registry entries tagged
`design_tool = figcraft` (`integrations.rs::design_tool_row`). An entry is `unknown` from the
moment it is saved (`orch-store::save_mcp_server` resets it) until one of three things updates it:

1. **The Test button** (`integration_test` / `mcp_server_test`): only when the user presses it.
2. **The background probe** (`registry.rs::probe_in_background`, at spawn and every 30 min in
   `probe_used_servers`): only for servers a running agent's role lists in `mcp_refs`. The sessions
   don't get FigCraft from the registry: they get it from the user's Claude Code config
   (`~/.claude.json` `mcpServers`, where it is named `figcraft` and `figcraft-app`). No role refs
   the entry, so it is never probed.
3. **The agents' tool calls** (`registry.rs::observe_mcp_call`, FR-604): a `PostToolUse` of
   `mcp__<server>__<tool>` marks the entry whose `tool_prefix()` is `<server>` connected. For a
   stdio entry that prefix is the registry **id**. The name Claude Code uses (the key in
   `~/.claude.json`) is the user's choice and need not be the registry id, so the calls the sessions
   make are not attributed to the FigCraft entry.

So nothing ever moves the entry off `unknown`, whatever the sessions do.

## Fix (backend only: `crates/agent-runtime`)

1. **Probe the design-tool entries on their own.** A FigCraft entry is a local stdio process: it
   can be probed for real (`probe_stdio`, `initialize` + `tools/list`).
   - At core start (on a thread, after startup), and in the 30-min sweep (`probe_used_servers`),
     also probe every **non-connector** registry entry tagged with a design tool, in every scope
     (global entries with `project = None`, each project override with its project), whether or not
     a role refs it. Same rules as today: skip one probed less than `PROBE_FRESH_MS` ago or being
     probed now (`probing` set).
   - After `mcp_server_save` of a non-connector entry tagged with a design tool, probe it in the
     background (the save reset it to `unknown`).
2. **Attribute the agents' calls by Claude Code's server name too.** A call to `mcp__<server>__*`
   also belongs to a **stdio** registry entry when Claude Code's config has a stdio server named
   `<server>` with the **same `command` and `args`** as the entry. Claude Code's config is
   `<home>/.claude.json`: its top-level `mcpServers` and the `mcpServers` of the agent's project
   (`projects[<repo path>]`). Read-only (never written, FR-1206), re-read only when its size or
   mtime changes (the same pattern as `orch-app/src/setup.rs` trust), `<home>` being the runtime's
   home (`crate::paths::home_dir()` or what tests inject). Matching by id stays first and unchanged;
   connectors are unchanged.
3. Unchanged: a probe that answers `unknown` never replaces a status the calls showed
   (`set_mcp_status`); `needs_auth` alerts; the wire (`Integration`, `McpServer` JSON). The UI
   needs no change: it already shows `connected` / `error` for FigCraft.

## Acceptance

1. A FigCraft stdio entry no role refs, never tested, becomes `connected` (detail `n tool(s)`,
   `checked_at_ms` set) shortly after the core starts, and Integrations shows FigCraft
   **Connected**; a command that fails shows **Error** with the probe's detail.
2. Saving a FigCraft entry probes it: its status leaves `unknown` without pressing Test.
3. With a registry entry `figcraft-cli` and a Claude Code server `figcraft` of the same
   command/args, an agent's `PostToolUse` of `mcp__figcraft__screenshot` marks the entry
   `connected` (detail `<agent> used screenshot`). A Claude Code server with other args (e.g.
   `figcraft-app`, `mcp --attach 7979`) is not attributed to it.
4. An entry probed less than `PROBE_FRESH_MS` ago isn't probed again by the start or the sweep.

## Tests (in `registry.rs` / `integrations.rs` tests, `test_runtime_net` + the fake stdio server of
`figcraft_is_probed_over_stdio`)

- The start/sweep probe of design-tool entries: an unreferenced FigCraft entry ends `connected`;
  a fresh one is skipped; a connector design-tool entry (Penpot) is not probed.
- Save → background probe.
- Call attribution through a fake `.claude.json` in the test home (top-level and per-project
  `mcpServers`), the same-args match and the other-args non-match; id match still wins.
- `cargo test -p agent-runtime` and `make` lint (fmt, clippy) pass.

## Out of scope

- Probing non-design-tool registry entries nobody refs.
- Importing Claude Code's servers into the registry, or any UI change.

## Team

- `back`: the whole fix (one task).
- `front`: reviews the backend task (no frontend work).
