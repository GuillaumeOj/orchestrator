# Add FigCraft Integration · API contract

No new command or event. Existing shapes gain one value each.

## 1. `Integration` (`integrations_list`, `integration_test`, …)

`items` has six entries, in this order: `github`, `linear`, `notion`, `penpot`, `figma`, **`figcraft`**.

```json
{
  "id": "figcraft",
  "name": "FigCraft",
  "status": "not_configured | connected | error | needs_auth | checking | unknown",
  "detail": "MCP: FigCraft, FigCraft (app)",
  "has_secret": false,
  "needs_secret": false,
  "checked_at_ms": 1762430000000,
  "account": null
}
```

- `status`: `not_configured` when no registry server (any scope) has `design_tool: "figcraft"`; else the worst of theirs (`needs_auth` > `error` > `connected` > `checking`, else `unknown`), as Penpot/Figma.
- `integration_test {id: "figcraft"}` probes every tagged server, returns the row.
- `integration_secret_set` / `integration_secret_remove` with `figcraft`: error `FigCraft doesn't take a token here` / `FigCraft has no token here`.

## 2. `McpServer.design_tool`

`null | "penpot" | "figma" | "figcraft"`. A typical FigCraft entry:

```json
{
  "id": "figcraft", "name": "FigCraft", "kind": "stdio",
  "command": "/Users/me/dev/figcraft/target/release/figcraft-cli", "args": ["mcp"],
  "allow_secret_env": false, "design_tool": "figcraft"
}
```

`args: ["mcp", "--attach", "7979"]` drives the running FigCraft app instead of a headless engine. Both are plain stdio servers: saved, tested and resolved into agents' `mcpServers` as today.

## 3. `Artifact.meta`

`tool`: `penpot | figma | figcraft | file | url`. For `figcraft`:

```json
{ "tool": "figcraft", "title": "Orchestrator.figcraft", "path": "design/app.figcraft",
  "node_id": "252198", "previewable": false, "on_integration": true }
```

- From a hand-off `artifact` = a repo path ending in `.figcraft` (case-insensitive), optional `#<node id>` or `#node=<node id>` (percent-decoded). `node_id` absent without a fragment.
- `on_integration` filled by the view as for `file`.
- `artifact_open` on it opens a copy extracted to the app's `previews/` folder (never the integration worktree's file) with the default app (FigCraft), not Quick Look. Errors unchanged (`not found on branch`).
