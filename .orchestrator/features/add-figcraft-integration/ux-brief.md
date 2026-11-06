# Add FigCraft Integration · UX brief

Small change inside existing screens; no new layout. Everything follows the Penpot/Figma rows and drafts.

## 1. Settings › Integrations

- Header line: "GitHub, Linear, Notion, Penpot, Figma, FigCraft" (the existing list, FigCraft appended).
- Row **FigCraft**, last. Icon: the same design-tool glyph the Penpot/Figma rows use (ui may pick a distinct one from the icon font; then front uses it).
- States (same badge as Penpot/Figma, `connector` style off: FigCraft is a local server, its status is real):
  - Not configured: badge "Not configured", detail "No FigCraft server in the MCP registry", action **Open MCP servers**.
  - Connected: detail "MCP: <server names>", checked time, actions **Test** and **Open MCP servers**.
  - Error: detail = the probe's message (e.g. "figcraft-cli: No such file or directory"), same actions.

## 2. Settings › MCP servers

- Design tool select: `—`, Penpot, Figma, **FigCraft**. Table column shows "FigCraft".
- Opened from the FigCraft row with no FigCraft server: a new **stdio** draft:
  - id `figcraft`, name `FigCraft`, design tool FigCraft, command empty with placeholder `figcraft-cli`, args `mcp`.
  - Hint line under the command field: "Full path to figcraft-cli. Args `mcp` run a headless engine; `mcp --attach 7979` drives the open FigCraft app."
- Opened with a FigCraft server: that server selected (as today for Penpot/Figma).

## 3. Design section (feature pane)

- A FigCraft hand-off tile: label "FigCraft", file name as title, `#node` shown after it when present ("Orchestrator.figcraft · 252198"). No preview; link-only treatment as Penpot/Figma (the designer's exported PNGs carry the preview). Open → FigCraft opens the file.

## 4. Keyboard

Nothing new; existing keys of both settings sections apply.
