# P3 — Ecosystem · UX brief

> For `ux` (flows, copy, states) and `ui` (Penpot frames, components). Spec: [spec.md](spec.md). Contract: [api-contract.md](api-contract.md).
> Process (ADR-008, SPEC §9.0): new screens are designed in the Penpot file **Orchestrator** (`65d5ee1e-9774-8045-8008-d16813b441c7`) before they're built. P3 frames go on a new row of each page (y = 6000 on `04 Screens`), marked **In review** on `00 Cover`. Only Guillaume sets **Ready for dev**.
> Use the existing tokens and P1/P2 components (§9.4). Same scenario as P1/P2 (Wednesday 21 Oct 11:00, shop-api › checkout-v2), extended with a `design-sprint` run on shop-api › "onboarding" for the design flow.

## Principles to keep
Keyboard-first (every new action in ⌘K and with a key), dense and calm, status never by color only, progressive disclosure (raw MCP config, tokens and payloads hidden until asked), and no secret ever shown back (a set secret reads "Stored in Keychain · Replace · Remove").

## Screens and states to design

| # | Screen / component | Page | States | Notes |
|---|---|---|---|---|
| 1 | **Settings pane** (new pane type, §9.3), sections: Claude Code · **Integrations** · **MCP servers** · Terminal/editor · Pacing policy · Keyboard · Memory. P3 designs only the two bold sections and the pane's frame/nav. | 04 | default, loading, integration disconnected | Left list of sections, right content. Opened with ⌘, and from ⌘K. |
| 2 | **Integration row** (component): icon, name, status badge (`connected / needs auth / error / not configured / checking`), checked-at, detail, actions Test · Connect/Reconnect · Remove | 02 | the five statuses, focus, busy | GitHub: "via gh · logged in as …". Linear/Notion: token field (write-only). |
| 3 | **Secret field** (component): write-only input with "Stored in Keychain" state | 02 | empty, typing, stored, error | Never shows the value. ⌘⏎ saves. |
| 4 | **MCP registry**: table of servers (name, kind, design tool, used by N roles, status) + editor drawer (kind-specific fields; env with `keychain:` refs and the **opt-in warning** for secrets in agent env; per-project override toggle) | 04 | empty, list, edit, connector hint, needs auth | Connector kind: explain that it comes from the user's claude.ai login and needs the role not to be strict. |
| 5 | **Role inspector — MCP picker and downgrade** (extends the P2 inspector) | 02/04 | none selected, selected with status dots, connector warning with strict MCP, downgrade off/on, downgrade warning on Lead/Reviewer | Downgrade: "to [sonnet ▾] when [Red ▾]", one line of cost copy ("switches at the next task; loses the prompt cache"). |
| 6 | **Feature pane — Design section** (extends P2 Feature): grid of design artifacts with preview thumbnail (exported images) or tool tile (Penpot/Figma link), from → to chips, note, time, Open | 04 | no designs yet, links only, previews, preview loading, preview too large / missing on branch, "not merged yet" (from the designer's branch) | Keys: `d` focus the Design section, arrows move, `⏎` open, `Space` large preview (overlay, Esc closes). |
| 7 | **Large preview overlay** | 04 | image, svg, error | No zoom tools beyond fit / 100 %. |
| 8 | **Presets**: preset card actions Duplicate (to global / to project) and "Save as preset"; "update available" hint on a modified starter copy | 02 | default, menu open, update available | Extends the P2 Preset card. Keys in the canvas: `D` duplicate. |
| 9 | **Week planner — Weekly report** (the J8 · 4 storyboard frame is the starting point): header (week, reset, final %), plan vs actual per allocation, breakdown tabs by project / feature / role / model, state time bar, governor actions summary, past reports list | 04 | current week (in progress, "so far"), past week, no data, estimated (low confidence) | Copy rules: pp of the week, 24 h times (Penpot 05). |
| 10 | **Model windows** in the planner and the pacing indicator: one row/chip per model window seen ("opus week 92 % · Red · resets Mon 09:00") | 02/04 | none (hidden), green, red, exhausted | Hidden when never seen. Queued explanation: "Queued: opus week in Red · starts after reset". |
| 11 | **Off-hours**: "Run off-hours" action (queued entry, task card menu, launch dialog checkbox), queued explanation "Off-hours · starts 19:00", planner curve shading for off-hours windows, **Not on AC** banner variant, power-assertion chip in the status bar ("☾ keeping awake") | 02/04/05 | queued off-hours, running off-hours, on battery | Banner copy in 05. |
| 12 | **Agent header — downgraded model** ("sonnet · downgraded (Red)") and its governor log entry with undo | 02 | — | Extends P1 pane header. |
| 13 | **Project settings — Task backend Linear**: backend select (Local / GitHub / Linear), Linear team picker, optional project, column mapping table (Linear state → column) with defaults, Sync now, last sync report | 04 | not connected (link to Integrations), loading teams, mapped, sync errors | |
| 14 | **Notion publish (C)**: on the Feature pane, "Publish to Notion" / "Refresh" with last published time and page link; Notion comment as an Inbox item (question kind, with "Open in Notion") | 04 | not configured, publishing, published, error | Lowest priority; design last. |
| 15 | **Inbox items**: MCP `needs auth` alert (with "Open Settings" and "Reconnect"), "Weekly report ready" digest entry, Linear sync conflict info ("remote won"), not-on-AC alert | 02 | — | Extend the P1 Inbox item variants. |

## Flows to storyboard (page 06, annotated frames with key sequences)
- **J12 — Design → code** (the P3 exit criterion): launch Design sprint on a feature → spec gate → UX brief (UX node) → UI frames in Penpot (UI node, Penpot MCP) → exported frames committed, `handoff` to Frontend → Lead merges → Feature page Design section shows previews → Frontend task starts and reads the images → PR gate. 6–8 frames over the existing 04 frames.
- **J13 — Connect an integration**: Settings → Integrations → Linear → paste token (⌘⏎) → Test → connected → Project settings → backend Linear → team → mapping → Sync now. 4–5 frames.
- **J8 · 4 (update)**: weekly report from the "Weekly report ready" Inbox item.

## Copy to write (page 05)
- Banners: "Not on AC power: off-hours work won't run while the Mac sleeps." / MCP: "Penpot MCP needs authentication: agents using it will fail its tools."
- Toasts: "Token stored in Keychain", "Duplicated Design sprint to shop-api", "Published 4 docs to Notion".
- Queued reasons: "Off-hours · starts 19:00", "opus week in Red · starts after reset Mon 09:00".
- Warnings: secret in agent env (FR-110 tone), downgrade on a Lead/Reviewer, connector with strict MCP.

## Keyboard additions (page 05)
- Global: ⌘, Settings.
- Feature pane: `d` Design section, `Space` preview, `N` publish to Notion (C).
- Canvas: `D` duplicate team/role.
- Week planner: `R` reports list.
- Board / queued entry: `O` run off-hours.
Check them against the P1/P2 maps for conflicts (the conflict checker FR-1204 reads the same table).

## Deliverables and hand-off
- `ux` → flows J12/J13, the copy and keys above, states per screen as annotations on the frames; notes in `docs/design/p3-design-notes.md`. Hands off to `ui` with `handoff`.
- `ui` → frames and components in Penpot, `00 Cover` statuses **In review**, exports in `docs/design/img/` (04-settings*.png, 04-feature-design.png, 04-weekly-report.png…), `docs/design/README.md` rows for P3, tokens unchanged unless a new token is needed (then also `docs/design/tokens.json`). Hands off frame links + exports to `front`.
- Frontend builds from the frames once Guillaume marks them Ready for dev; until then front builds against the contract with plain layouts (backport rule, ADR-008).
