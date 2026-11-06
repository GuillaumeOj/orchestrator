# Add FigCraft Integration · Feature spec

> Feature `add-figcraft-integration` · Integration branch `orch/add-figcraft-integration/integration` · core (Rust) + Settings UI + a small backport to the FigCraft file

## 1. Goal

Settings › **Integrations** lists GitHub, Linear, Notion, Penpot and Figma. FigCraft, the design tool the app's own UI is designed in (ADR-012), is missing. Add **FigCraft** as a sixth integration, on the same footing as Penpot and Figma: a design tool reached through MCP servers in the registry, with a status, a test, and a place in the design hand-off flow.

## 2. What exists today

- `orch_team::mcp::DesignTool { Penpot, Figma }` (serde lowercase) tags a registry entry (`McpServer.design_tool`). Settings › MCP servers has a "design tool" select (`—`, Penpot, Figma).
- `agent-runtime/src/integrations.rs`: `IDS = [github, linear, notion, penpot, figma]`. A design-tool integration takes no token (`needs_secret` false); its status is the worst status of the registry servers tagged with that tool, in any scope (`design_tool_row`); its test probes those servers (`mcp_server_test`).
- The UI (`IntegrationsSettings.svelte`) shows design-tool rows with the action **Open MCP servers**, which opens `settings/mcp` at `design:<tool>`: the first server tagged with the tool, or a new draft (`McpSettings.svelte` `startNew`, today always a `connector` draft).
- Hand-offs (`orch_team::artifact::parse`): Penpot and Figma links become `meta.tool = penpot | figma` (link-only tiles); a repo path is `file` (previewed if an image, opened in Quick Look).
- FigCraft's MCP server is a **local stdio** process: `figcraft-cli mcp` (headless engine) or `figcraft-cli mcp --attach 7979` (drives the running FigCraft app). Unlike the Penpot/Figma claude.ai connectors it can be probed (the stdio `initialize` probe already exists). FigCraft files (`*.figcraft`) live in the repo (here [`Orchestrator.figcraft`](https://drive.google.com/open?id=1n4O57ntQDFvv1-pw3flJkmcHh4MeTLVs&usp=drive_fs), Git LFS).

## 3. Scope

### 3.1 Core (back, T-1)

1. **`DesignTool::Figcraft`** (`"figcraft"` on the wire), in `crates/orch-team/src/mcp.rs`. No seed entry: the server's command is a local path, unknown to the app (Penpot stays the only seed).
2. **Integration `figcraft`** in `crates/agent-runtime/src/integrations.rs`: appended to `IDS` (display order: … Penpot, Figma, **FigCraft**), name `"FigCraft"`, no secret, `design_tool("figcraft") = Figcraft`. Status, detail (`MCP: <names>`), `checked_at_ms` and test work exactly as for Penpot/Figma (`design_tool_row`, `integration_test`); the stdio probe gives a real `connected` / `error` status.
3. **Hand-offs of FigCraft frames** (`crates/orch-team/src/artifact.rs`): a repo-relative path ending in `.figcraft` (case-insensitive), optionally followed by `#<node id>`, parses to `meta.tool = "figcraft"`, `path` (without the fragment), `node_id` (the fragment, percent-decoded; `node=` prefix accepted), `title` (file name), `previewable: false`. Today's rules for the path (no `..`, not absolute…) are unchanged. Any other path stays `file`.
4. **Opening a FigCraft artifact** (`crates/agent-runtime/src/artifacts.rs`): `artifact_open_target` always extracts a `figcraft` artifact's blob (integration branch, else the designer's branch) into the app's `previews/` folder, never the integration worktree's own file (FigCraft saving there would dirty it; Open is for viewing, edits to the copy are not kept), and returns a new `OpenTarget::App(PathBuf)`, which `artifact_open` runs with `/usr/bin/open <path>` (the app that owns `.figcraft`, i.e. FigCraft) instead of Quick Look. Not found → the existing `not found on branch` error. Note: a file extracted from a branch is the LFS pointer if the blob is in LFS; extraction then runs the bytes through `git lfs smudge` when they are a pointer (`version https://git-lfs…`), or fails with `not found on branch` if `git lfs` is absent. Owner verifies on the real repo.
5. **Design hand-off prompt** (`orch_team::presets::DESIGN_EXPORTS`): leave unchanged (changing starter prompt text shifts starter fingerprints/upgrades; out of scope, §5).
6. **Docs**: `docs/SPEC.md` FR-1203 lists FigCraft; `crates/agent-runtime/README.md` and `crates/orch-team/README.md` mention FigCraft where they list Penpot/Figma.
7. **Tests**: DesignTool round-trip; `integrations_json` has 6 items in order with `figcraft` `not_configured` and `needs_secret: false`; a stdio server tagged `figcraft` makes the integration `connected`/`error` from its probe (reuse the existing fake stdio test setup); `integration_secret_set("figcraft", …)` refused; artifact parse cases (`a/b.figcraft`, `a/b.FigCraft#252198`, `a/b.figcraft#node=252198`, `../x.figcraft` → None, `a.figcraft.png` → file); open target is `App` for a figcraft artifact.

### 3.2 Settings UI (front, T-3)

Contract: [api-contract.md](api-contract.md). Copy and states: [ux-brief.md](ux-brief.md). Frames: the T-2 exports under `design/`.

- `IntegrationsSettings.svelte`: the design-tool check includes `figcraft`; the header list and the row icon include FigCraft. Its row behaves as Penpot/Figma (Open MCP servers, Test).
- `McpSettings.svelte`: design-tool select gains **FigCraft**; the table's Design tool column shows "FigCraft"; `design:figcraft` with no tagged server starts a **stdio** draft: id `figcraft`, name `FigCraft`, kind `stdio`, command placeholder `figcraft-cli` (empty value: the user enters the full path), args `mcp`, design tool `figcraft`, with the hint line from the UX brief.
- `lib/ecosystem.js`: off copy `figcraft: { off: 'No FigCraft server in the MCP registry' }`; `TOOL.figcraft` (label `FigCraft`); `artifactTool` guesses `figcraft` for a URI whose path ends in `.figcraft` (before the `#`); `linkOnly` includes `figcraft` (no image preview: the tile says to look at the exports); a FigCraft tile's title is "<file> · <node_id>" when `meta.node_id` is set (UX brief §3).
- Unit tests in `ui/test` for the ecosystem helpers and, where the existing tests cover them, the Integrations row and the new draft.

### 3.3 FigCraft file backport (ui, T-2)

SPEC §9.0: a code change is backported to the FigCraft file in the same PR.
- Page `04.7 Settings`: Integrations (FigCraft row in its Not configured and Connected states) and MCP servers (FigCraft in the design-tool select, the new FigCraft stdio draft with its hint).
- Export the changed frames to `.orchestrator/features/add-figcraft-integration/design/*.png`; refresh `docs/design/img/04-settings-integrations.png`; one line in `docs/design/README.md`'s status table.
- Hand off the exports to `front`.

## 4. Acceptance

1. Settings › Integrations shows six rows; FigCraft is last, `Not configured` with no tagged server, copy as in the UX brief.
2. Open MCP servers on FigCraft opens a stdio draft prefilled as in §3.2; saving it with a valid `figcraft-cli` path and Test shows `Connected` on both the server and the integration row.
3. A server whose command fails shows `Error` with the probe's detail on both.
4. A `ui` hand-off of `https://drive.google.com/open?id=1n4O57ntQDFvv1-pw3flJkmcHh4MeTLVs&usp=drive_fs#252198` appears as a FigCraft tile; Open launches FigCraft on that file.
5. `cargo test`, `cargo clippy`, `bun test` and `biome` pass; CI is green.

## 5. Out of scope

- A FigCraft seed in the registry, or in the design starter roles' `mcp_refs` (stays `penpot`).
- Changing the starters' hand-off prompt text.
- Deep-linking FigCraft to a node on open (FigCraft has no URL scheme we know of; the node id stays in `meta.node_id` for later).
- Downgrade safety: a registry entry tagged `figcraft` can't be read by a build without this feature (the `design_tool` field fails to deserialize). Same as any new enum value; noted, not handled.

## 6. Tasks

| Task | Node | Depends on | Files |
|---|---|---|---|
| T-1 Core: DesignTool, integration, artifact parse/open, docs | back | — | `crates/orch-team/src/{mcp,artifact}.rs`, `crates/agent-runtime/src/{integrations,artifacts}.rs`, READMEs, `docs/SPEC.md` |
| T-2 FigCraft backport + exports | ui | — | [`Orchestrator.figcraft`](https://drive.google.com/open?id=1n4O57ntQDFvv1-pw3flJkmcHh4MeTLVs&usp=drive_fs), `docs/design/img/`, `docs/design/README.md`, `design/` |
| T-3 Settings UI | front | T-2 (merged first) | `ui/src/components/{IntegrationsSettings,McpSettings}.svelte`, `ui/src/lib/ecosystem.js`, `ui/test/` |

No UX task: the copy and states fit in [ux-brief.md](ux-brief.md).
